//! Lost sensors, inputs that keep a feed from starting, reversals and the
//! food beam, through the owner.
//!
//! Lost sensors (an ADC conversion that never completes) are held, not
//! reset on: the owner keeps its fault with the outputs verified stopped
//! and goes on beating, so the platform stays up and says why it cannot
//! feed. A lost supervisor still lapses the beat into a watchdog reset.

use crate::common::owner::*;
use crate::common::*;
use wellfed_core::feed::controller::FeedReason;
use wellfed_core::feed::storage::{OwnerOutcome, OwnerRecovery};
use wellfed_hal::board::pins::{FOOD, OUTLET, bit};
use wellfed_hal::sdk::fake;
use wellfed_hal_feeder::board::MOTOR_MASK;
use wellfed_hal_feeder::owner::{OwnerConfig, OwnerFault, OwnerState, SubmitResult};

/// Steps for longer than the rest gap, and the beats published meanwhile.
fn beats_over_the_gap(f: &mut Fixture) -> u32 {
    let before = beats();
    f.advance(3 * CONFIG.idle_gap_ms);
    beats() - before
}

fn boot_with(config: impl FnOnce(&mut OwnerConfig)) -> Box<Fixture> {
    fake::reset_hardware();
    let mut f = Box::new(Fixture::new(PHASE_KNOWN));
    config(&mut f.config);
    f.create();
    f.step();
    assert_eq!(f.status().state, OwnerState::Idle);
    f
}

#[test]
fn lost_sensors_at_rest_are_held_with_the_outputs_stopped() {
    let mut f = boot();
    hw(|h| h.stuck = true);

    // The next battery conversion finds it, a rest period away at most.
    f.advance(CONFIG.rest_capture_ms);
    let s = f.status();
    assert!(s.state == OwnerState::Fault && s.fault == OwnerFault::Sensor);
    assert!(s.stopped && s.optics_off && motor_pins() == MOTOR_MASK);

    // Beating on, past the gap a lapsed beat would reset at.
    assert!(beats_over_the_gap(&mut f) > 0);
    assert_eq!(f.owner.submit(1, 1), SubmitResult::RejectStopped);
    assert_eq!(f.reserve_calls, 0);
    f.stop();
}

#[test]
fn lost_sensors_during_a_feed_fail_it_as_a_sensor_fault_and_are_held() {
    let mut f = boot();
    f.start_feed();
    hw(|h| h.stuck = true);

    f.step();
    let s = f.status();
    assert!(s.state == OwnerState::Fault && s.fault == OwnerFault::Sensor);
    assert!(s.result_durable && s.result.outcome == OwnerOutcome::Failed);
    assert_eq!(s.result.reason, FeedReason::Sensor);
    assert!(s.stopped && s.optics_off && f.finish_calls == 1);

    assert!(beats_over_the_gap(&mut f) > 0);
    assert_eq!(f.finish_calls, 1, "recorded once");
    f.stop();
}

#[test]
fn a_lost_supervisor_still_lapses_the_beat() {
    let mut f = boot();
    assert_eq!(f.owner.submit(1, 1), SubmitResult::Enqueued);
    f.step();
    f.advance(2); // The emitters on, still preparing.

    // The owner misses its lease with the emitters on: the timer cuts them
    // and faults the supervisor, which a restart must clear.
    fake::advance(12 * 32, true);
    f.step();
    let s = f.status();
    assert!(s.state == OwnerState::Fault && s.fault == OwnerFault::Hardware);
    assert!(s.stopped && s.optics_off);
    assert_eq!(beats_over_the_gap(&mut f), 0);
}

#[test]
fn an_outlet_blocked_before_the_drive_fails_the_feed_with_no_motion() {
    let mut f = boot();
    hw(|h| h.input[0] &= !bit(OUTLET));

    assert_eq!(f.owner.submit(1, 1), SubmitResult::Enqueued);
    for _ in 0..=CONFIG.prepare_timeout_ms {
        f.advance(1);
        assert_eq!(motor_pins(), MOTOR_MASK, "never driven");
    }
    let s = f.status();
    assert!(s.state == OwnerState::Terminal && s.fault == OwnerFault::Ok);
    assert!(s.result_durable && !s.result.motion_possible);
    assert_eq!(
        (s.result.outcome, s.result.reason),
        (OwnerOutcome::Failed, FeedReason::Outlet)
    );

    // Nothing latched: with the outlet clear, the next feed runs.
    hw(|h| h.input[0] |= bit(OUTLET));
    assert!(f.owner.acknowledge(1));
    f.advance(1);
    assert_eq!(f.owner.submit(2, 1), SubmitResult::Enqueued);
    f.advance(5);
    assert_eq!(f.status().state, OwnerState::Feeding);
    f.stop();
}

/// An outlet that blocks during the drive ends the feed: the motor stops
/// within the outlet filter's delay, the result is durable and names the
/// outlet, and the outlet clearing again resumes nothing.
#[test]
fn an_outlet_blocked_during_the_drive_stops_it_for_good() {
    let mut f = boot();
    f.start_feed();
    f.advance(2);

    hw(|h| h.input[0] &= !bit(OUTLET));
    f.advance(CONFIG.sensors.outlet_block_delay_ms + 2);
    let s = f.status();
    assert_eq!(motor_pins(), MOTOR_MASK, "stopped");
    assert!(s.state == OwnerState::Terminal && s.result_durable);
    assert!(s.result.motion_possible);
    assert_eq!(
        (s.result.outcome, s.result.reason, s.result.completed),
        (OwnerOutcome::Failed, FeedReason::Outlet, 0)
    );

    hw(|h| h.input[0] |= bit(OUTLET));
    for _ in 0..20 {
        f.advance(1);
        assert_eq!(motor_pins(), MOTOR_MASK, "never resumed");
    }
    assert_eq!(f.status().state, OwnerState::Terminal);
    f.stop();
}

#[test]
fn a_feed_that_only_ran_out_of_time_still_expires() {
    let mut f = boot();
    f.reserve_delay_ticks = 25 * 32;

    assert_eq!(f.owner.submit(1, 1), SubmitResult::Enqueued);
    f.step();
    let s = f.status();
    assert_eq!(
        (s.result.outcome, s.result.reason),
        (OwnerOutcome::Expired, FeedReason::TotalTimeout)
    );
    f.stop();
}

#[test]
fn a_jam_backed_off_is_traced_and_the_feed_completes() {
    let mut f = boot_with(|c| c.feed.max_reverse_attempts = 1);
    f.start_feed();

    // About 375 scaled, over the 300 limit, for two polls; then the pins
    // go back (PA14 alone), stop, and forward again (PA13 alone).
    hw(|h| h.current_adc = 700);
    f.advance(2);
    hw(|h| h.current_adc = 500);
    let mut pins = Vec::new();
    for _ in 0..12 {
        f.advance(1);
        if pins.last() != Some(&motor_pins()) {
            pins.push(motor_pins());
        }
    }
    assert_eq!(pins, [MOTOR_MASK, 1 << 14, MOTOR_MASK, 1 << 13]);

    fake::advance(32, true);
    fake::edge(true);
    f.step();
    let s = f.status();
    assert!(s.state == OwnerState::Terminal && s.result.outcome == OwnerOutcome::Done);
    assert_eq!(s.trace.reversals, 1);
    f.stop();
}

#[test]
fn the_food_beam_reading_outlasts_the_feed_that_took_it() {
    let mut f = boot();
    assert_eq!(f.status().food, None, "the beam is read only lit");

    // The fake's food receiver reads LOW: food in front of the beam.
    f.start_feed();
    assert_eq!(f.status().food, Some(true));
    f.complete();
    let s = f.status();
    assert!(s.state == OwnerState::Terminal && s.optics_off);
    assert_eq!(s.food, Some(true));
    assert_eq!(s.trace.hopper, Some(true), "the feed read only food");
    f.stop();
}

/// A feed runs whatever the food beam reads, as stock's does, so a beam
/// that misreads never skips a meal; its trace keeps that it read empty.
#[test]
fn a_feed_on_an_empty_hopper_completes_and_its_trace_says_it_read_empty() {
    let mut f = boot();
    // The food receiver reads HIGH: no food in front of the beam.
    hw(|h| h.input[0] |= bit(FOOD));

    f.start_feed();
    f.complete();
    let s = f.status();
    assert!(s.state == OwnerState::Terminal && s.result_durable);
    assert_eq!(
        (s.result.outcome, s.result.completed),
        (OwnerOutcome::Done, 1)
    );
    assert_eq!(s.food, Some(false));
    assert_eq!(s.trace.hopper, Some(false));
    f.stop();
}

#[test]
fn an_owner_that_resolved_a_feed_and_lost_its_sensors_holds_the_result() {
    fake::reset_hardware();
    let mut f = Box::new(Fixture::new(OwnerRecovery {
        high_water_id: 7,
        pending_request: true,
        ..PHASE_KNOWN
    }));
    f.create();
    f.step();
    assert_eq!(f.status().state, OwnerState::Terminal);

    hw(|h| h.stuck = true);
    assert!(f.owner.acknowledge(7));
    f.advance(2);
    let s = f.status();
    assert!(s.state == OwnerState::Fault && s.fault == OwnerFault::Sensor);
    assert!(s.result.id == 7 && s.result_durable && f.resolve_calls == 1);
    assert!(beats_over_the_gap(&mut f) > 0);
    f.stop();
}

/// At rest the converters run far less often than the owner polls, and nothing
/// that keeps the feeder safe waits for them.
mod at_rest {
    use super::*;
    use wellfed_core::adc_scale::SAMPLES;
    use wellfed_hal::board::pins::{ADAPTER, ADC_BATTERY};
    use wellfed_hal::power;
    use wellfed_hal::sdk;

    const PERIOD_MS: u32 = 10;
    /// One channel's conversions, as the fake counts its reads.
    const CHANNEL: u32 = SAMPLES as u32;
    /// About 1619 scaled: under the floor, above the cutoff.
    const UNDER_FLOOR_RAW: u16 = 2200;

    /// An owner whose rest period is long enough to poll between conversions.
    fn spaced() -> Box<Fixture> {
        boot_with(|c| {
            c.sensors.max_age_ms = PERIOD_MS;
            c.rest_capture_ms = PERIOD_MS;
        })
    }

    fn conversions() -> u32 {
        hw(|h| h.reads)
    }

    /// `spaced`, settled on the pack alone.
    fn on_the_pack() -> Box<Fixture> {
        let mut f = spaced();
        hw(|h| h.input[0] &= !bit(ADAPTER));
        f.advance(2 * PERIOD_MS);
        assert!(!f.status().on_adapter && !f.status().supply_low);
        f
    }

    #[test]
    fn only_the_battery_converts_and_only_at_its_period() {
        let mut f = spaced();
        let before = conversions();

        f.advance(3 * PERIOD_MS);
        assert_eq!(conversions() - before, 3 * CHANNEL);
        assert_eq!(hw(|h| h.adc_channel), ADC_BATTERY);
        f.stop();
    }

    #[test]
    fn the_owner_beats_on_every_poll_between_conversions() {
        let mut f = spaced();
        f.advance(PERIOD_MS);
        let before = beats();

        f.advance(PERIOD_MS - 1);
        assert_eq!(beats() - before, PERIOD_MS - 1);
        f.stop();
    }

    /// The lease is what lets a food reading's emitters stay lit: a poll with
    /// no conversion must renew it just as one with a conversion does.
    #[test]
    fn the_owner_lease_is_renewed_between_conversions() {
        let mut f = spaced();
        for _ in 0..3 * CONFIG.motor_lease_ms {
            f.advance(1);
            let now = f.owner.events().now_ms().expect("the clock reads");
            assert_eq!(f.owner.events().view().owner_at, now);
        }
        assert_eq!(f.status().fault, OwnerFault::Ok);
        f.stop();
    }

    #[test]
    fn the_inputs_are_read_on_every_poll() {
        let mut f = spaced();
        f.advance(PERIOD_MS);
        assert!(f.status().on_adapter);

        // Right after a conversion, so the next is a whole period away.
        hw(|h| h.input[0] &= !bit(ADAPTER));
        let before = conversions();
        f.advance(1);
        assert!(!f.status().on_adapter);
        assert_eq!(conversions(), before, "with nothing converted");
        f.stop();
    }

    #[test]
    fn a_feed_converts_both_channels_from_its_first_poll_to_its_last() {
        let mut f = spaced();
        f.advance(1);
        let before = conversions();

        assert_eq!(f.owner.submit(1, 1), SubmitResult::Enqueued);
        f.step();
        assert_eq!(f.status().state, OwnerState::Preparing);
        assert_eq!(conversions() - before, 2 * CHANNEL);

        for _ in 0..5 {
            let before = conversions();
            f.advance(1);
            assert_eq!(conversions() - before, 2 * CHANNEL);
        }
        assert_eq!(f.status().state, OwnerState::Feeding);
        let before = conversions();
        f.advance(1);
        assert_eq!(conversions() - before, 2 * CHANNEL);

        f.complete();
        assert!(f.owner.acknowledge(1));
        f.advance(1);
        let before = conversions();
        f.advance(PERIOD_MS);
        assert_eq!(conversions() - before, CHANNEL, "back to resting");
        f.stop();
    }

    /// A feed a poll after a conversion starts on the same readings as any:
    /// the current it checks at the start is converted, not left out.
    #[test]
    fn a_motor_current_over_the_limit_still_refuses_a_feed_from_rest() {
        let mut f = spaced();
        hw(|h| h.current_adc = 700);
        f.advance(PERIOD_MS);
        assert_eq!(f.status().state, OwnerState::Idle, "not judged at rest");

        assert_eq!(f.owner.submit(1, 1), SubmitResult::Enqueued);
        f.advance(CONFIG.prepare_timeout_ms + 1);
        let s = f.status();
        assert_eq!(
            (s.result.outcome, s.result.reason),
            (OwnerOutcome::Failed, FeedReason::Sensor)
        );
        assert_eq!(motor_pins(), MOTOR_MASK, "never driven");
        f.stop();
    }

    #[test]
    fn the_pack_keeps_its_verdict_and_its_sample_between_conversions() {
        let mut f = on_the_pack();
        for _ in 0..3 * PERIOD_MS {
            f.advance(1);
            let s = f.status();
            assert!(!s.supply_low && s.below_floor == Some(false));
            let sample = power::latest(sdk::uptime_ms()).expect("a published sample");
            assert!(
                sample.pin_mv > 0,
                "a poll with no conversion publishes none"
            );
        }
        f.stop();
    }

    /// The floor's hold is timed: at rest two conversions a rest period apart
    /// span a hold shorter than it, where a count of polls would have seen one
    /// reading.
    #[test]
    fn a_sag_at_rest_is_judged_on_the_time_it_lasted() {
        const { assert!(PERIOD_MS >= CONFIG.sensors.battery.hold_ms) };
        let mut f = on_the_pack();

        hw(|h| h.battery_adc = UNDER_FLOOR_RAW);
        f.advance(PERIOD_MS);
        assert_eq!(f.status().below_floor, Some(false), "one reading under");
        f.advance(PERIOD_MS);
        let s = f.status();
        assert!(s.below_floor == Some(true) && s.supply_low);
        f.stop();
    }

    /// A gap past the observation limit loses the battery's reading, and the
    /// next poll converts again whatever its period says, before anything can
    /// read the pack as low for want of one.
    #[test]
    fn a_held_off_owner_converts_again_at_once() {
        let mut f = on_the_pack();
        f.advance(PERIOD_MS);

        fake::advance((CONFIG.sensors.max_observation_gap_ms + 1) * 32, true);
        f.step();
        assert!(!f.status().supply_low, "the supply is unknown, not low");

        let before = conversions();
        f.advance(1);
        assert_eq!(conversions() - before, CHANNEL);
        assert!(!f.status().supply_low);
        f.stop();
    }

    #[test]
    fn the_rest_period_stays_inside_the_limits_it_answers_to() {
        let valid = |change: fn(&mut OwnerConfig)| {
            let mut config = CONFIG;
            change(&mut config);
            config.valid()
        };

        assert!(valid(|_| {}));
        assert!(!valid(|c| c.rest_capture_ms = c.sample_period_ms - 1));
        assert!(!valid(|c| c.rest_capture_ms = c.sensors.max_age_ms + 1));
        assert!(!valid(|c| {
            c.sensors.max_age_ms = c.motor_lease_ms;
            c.rest_capture_ms = c.motor_lease_ms;
        }));
    }
}

mod food_reads {
    use super::*;
    use wellfed_core::feed::food_read::{FOOD_READ_MS, FoodReads};
    use wellfed_hal::board::pins::ADAPTER;
    use wellfed_hal::interlock::Interlock;
    use wellfed_hal::ota::mailbox::{self, Request};
    use wellfed_hal::sdk;
    use wellfed_hal::stopped;
    use wellfed_hal::supervision::beat;
    use wellfed_hal::supervision::supervise::{self, Inputs, Startup};
    use wellfed_hal_feeder::events::{EventsFault, OpticsFor};

    /// An owner settled on the adapter, its beam reading `food`.
    fn at_rest(food: bool) -> Box<Fixture> {
        let mut f = boot();
        hw(|h| {
            h.input[0] |= bit(ADAPTER);
            if !food {
                h.input[0] |= bit(FOOD);
            }
        });
        f.advance(10);
        assert!(f.status().on_adapter);
        f
    }

    /// What the runtime's watchdog feeder would decide now, the role started.
    fn watchdog_fed() -> bool {
        let outputs = stopped::observe();
        supervise::feed(&Inputs {
            now_ms: sdk::uptime_ms(),
            startup: Startup::Started,
            tasks_alive: true,
            outputs_ok: outputs.verified() || outputs.sensing(),
            owner: beat::read(),
            ..Inputs::default()
        })
    }

    #[test]
    fn a_window_reads_low_without_touching_the_motor_or_the_watchdog() {
        let mut f = at_rest(false);
        assert!(f.owner.read_food());
        assert!(f.status().food_reading && !f.status().optics_off);
        assert!(stopped::observe().sensing(), "reported as sensing");
        assert!(!stopped::observe().verified());

        for _ in 0..FOOD_READ_MS - 1 {
            f.advance(1);
            assert_eq!(motor_pins(), MOTOR_MASK);
            assert!(stopped::observe().sensing());
            assert!(watchdog_fed());
            assert_eq!(f.status().food, None, "judged at the window's end");
            assert_eq!(f.status().state, OwnerState::Idle);
        }
        f.advance(2);

        let s = f.status();
        assert_eq!((s.food, s.food_reads), (Some(false), 1));
        assert!(!s.food_reading && s.optics_off && s.stopped);
        assert!(stopped::observe().verified() && !stopped::observe().sensing());
        assert_eq!(s.trace.hopper, None, "no feed's trace");
        assert!(watchdog_fed());
        f.stop();
    }

    #[test]
    fn a_flicker_inside_a_window_is_no_change() {
        let mut f = at_rest(true);
        assert!(f.owner.read_food());
        // Food, then none for longer than the filter's delay here, then food.
        f.advance(100);
        hw(|h| h.input[0] |= bit(FOOD));
        f.advance(100);
        hw(|h| h.input[0] &= !bit(FOOD));
        assert_eq!(f.status().food, None);
        f.advance(FOOD_READ_MS);
        assert_eq!(f.status().food, Some(true));
        f.stop();
    }

    /// The filter on its way down at the window's end says neither.
    #[test]
    fn a_window_that_ends_between_levels_settles_nothing() {
        let mut f = at_rest(true);
        assert!(f.owner.read_food());
        f.advance(FOOD_READ_MS - 3);
        hw(|h| h.input[0] |= bit(FOOD));
        f.advance(5);
        let s = f.status();
        assert!(!s.food_reading && s.optics_off);
        assert_eq!((s.food, s.food_reads), (None, 0));
        f.stop();
    }

    #[test]
    fn no_reading_starts_on_the_pack_or_with_the_owner_busy() {
        let mut f = boot();
        hw(|h| h.input[0] &= !bit(ADAPTER));
        f.advance(10);
        assert!(!f.status().on_adapter);
        assert!(!f.owner.read_food(), "on the pack");
        assert!(optics_off());

        hw(|h| h.input[0] |= bit(ADAPTER));
        f.advance(10);
        assert_eq!(f.owner.submit(1, 1), SubmitResult::Enqueued);
        assert!(!f.owner.read_food(), "a feed is queued");
        f.step();
        assert!(!f.owner.read_food(), "a feed is preparing");
        f.stop();
    }

    #[test]
    fn no_reading_starts_while_an_update_holds_the_flash_or_a_lease_runs() {
        let mut f = at_rest(false);
        mailbox::reset();
        mailbox::request(Request::offer(&[0; 160], None, [0; 32]).unwrap()).unwrap();
        assert!(!f.owner.read_food(), "an install");

        // The loop's half holds it too, with no interlock flag left behind.
        let interlock = Interlock::new();
        let mut reads = FoodReads::new();
        for now in (0..30_000).step_by(20) {
            f.owner.food_read_step(&mut reads, &interlock, now, false);
            assert!(optics_off() && !f.status().food_reading);
        }
        assert!(!interlock.holds_reading());
        assert!(interlock.begin_reading(10), "the flag was never left set");
        interlock.end_reading();

        mailbox::finish(Err(wellfed_core::ota::install::Refused::Busy));
        mailbox::reset();

        // A lease in progress: refused, with no fault latched.
        f.hold();
        assert!(!f.owner.read_food(), "a lease");
        assert_eq!(f.status().fault, OwnerFault::Ok);
        assert_eq!(f.owner.events().view().fault, EventsFault::Ok);
        f.release_hold();
        f.stop();
    }

    /// A feed the owner takes during a reading wins: the emitters drop before
    /// its journal write, the reading is not judged, and the feed runs.
    #[test]
    fn a_feed_during_a_window_wins() {
        let mut f = at_rest(false);
        assert!(f.owner.read_food());
        f.advance(50);

        assert_eq!(f.owner.submit(1, 1), SubmitResult::Enqueued);
        // The storage callback asserts the emitters are off as it reserves.
        f.step();
        assert!(f.status().intent_reserved);
        assert!(!f.status().food_reading);
        assert!(!stopped::observe().sensing());
        f.advance(5);
        assert_eq!(f.status().state, OwnerState::Feeding);
        assert_eq!(motor_pins(), 1 << 13);
        assert_eq!(f.status().food_reads, 0);
        f.complete();
        assert_eq!(f.status().result.outcome, OwnerOutcome::Done);
        f.stop();
    }

    /// The loop can end a reading between steps, which a feed's claim relies on.
    #[test]
    fn the_loop_ends_a_window_at_once_and_a_later_step_judges_nothing() {
        let mut f = at_rest(false);
        let interlock = Interlock::new();
        assert!(interlock.begin_reading(10));
        assert!(f.owner.read_food());
        f.advance(FOOD_READ_MS - 10);

        f.owner.yield_food_read(&interlock);
        assert!(optics_off() && stopped::observe().verified());
        assert!(!stopped::observe().sensing());
        assert!(!f.status().food_reading && f.status().optics_off);
        assert!(interlock.begin_reading(10), "the flag was handed back");
        interlock.end_reading();

        f.advance(FOOD_READ_MS);
        let s = f.status();
        assert_eq!((s.food, s.food_reads), (None, 0));
        assert_eq!(s.state, OwnerState::Idle);
        f.stop();
    }

    /// A stale owner lease cuts a reading from the timer interrupt alone, with
    /// nothing latched, where a feed's emitters latch the OWNER fault.
    #[test]
    fn a_held_off_owner_loses_the_reading_and_nothing_else() {
        let mut f = at_rest(false);
        assert!(f.owner.read_food());
        for _ in 0..CONFIG.motor_lease_ms + 2 * CONFIG.events.timer_period_ms {
            fake::advance(32, true);
        }
        assert!(optics_off(), "cut with no step");
        assert_eq!(f.owner.events().view().fault, EventsFault::Ok);

        f.advance(5);
        let s = f.status();
        assert!(s.state == OwnerState::Idle && s.fault == OwnerFault::Ok);
        assert_eq!(s.supervisor_fault, EventsFault::Ok);
        assert!(!s.food_reading && s.optics_off);
        assert_eq!((s.food, s.food_reads), (None, 0));
        assert!(!stopped::observe().sensing());
        f.stop();
    }

    /// The supervisor's budget holds with the owner renewing its lease: the
    /// emitters go off at the budget, whatever the owner meant to do.
    #[test]
    fn the_supervisor_cuts_a_reading_at_its_budget() {
        let mut f = at_rest(false);
        assert!(f.owner.events().optics_for_reading(20));
        f.advance(10);
        assert!(!optics_off());
        f.advance(15);
        assert!(optics_off(), "cut at the budget");
        assert_eq!(f.owner.events().view().fault, EventsFault::Ok);
        f.stop();
    }

    /// After a reading, a feed's own emitters are a feed's: a stale lease then
    /// latches a fault as before.
    #[test]
    fn a_feed_s_emitters_after_a_reading_still_latch_a_fault() {
        let mut f = at_rest(true);
        assert!(f.owner.read_food());
        f.advance(FOOD_READ_MS + 2);
        assert_eq!(f.status().food, Some(true));

        f.start_feed();
        assert_eq!(f.owner.events().view().optics_for, OpticsFor::Feed);
        for _ in 0..CONFIG.motor_lease_ms + 2 * CONFIG.events.timer_period_ms {
            fake::advance(32, true);
        }
        assert_ne!(f.owner.events().view().fault, EventsFault::Ok);
    }
}
