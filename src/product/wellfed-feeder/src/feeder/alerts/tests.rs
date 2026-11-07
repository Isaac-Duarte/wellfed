extern crate std;

use std::vec::Vec;

use wellfed_proto::api::{FeedReason, MissedReason, Path, Role};

use super::*;

/// A wall clock well past zero, in Unix seconds.
const WALL: u32 = 1_800_000_000;
const CLIENT: [u8; 4] = [1, 2, 3, 4];
const WATCHDOG: u32 = 1 << 17;

fn calm() -> Readings {
    Readings {
        wall_s: Some(WALL),
        zoned: true,
        blocked: None,
        food: Some(true),
        feeding: false,
        desiccant: Desiccant::Remaining { days: 14 },
        supply: Supply {
            adapter: Some(true),
            battery: Battery::Absent,
            below_floor: None,
        },
        claim: false,
        plans_settled: true,
    }
}

/// Rules with the boot read done and everything well.
fn settled() -> Alerts {
    let mut alerts = Alerts::new();
    alerts.restored(0, History::Whole);
    alerts.step(0, &calm());
    assert!(!alerts.settling());
    alerts
}

fn kinds(alerts: &Alerts) -> Vec<AlertKind> {
    alerts.open().map(|open| open.alert.kind()).collect()
}

fn fed(feed: u32, outcome: FeedOutcome, reason: FeedReason, by: Source) -> EventBody {
    EventBody::Fed {
        feed,
        requested: 2,
        completed: 1,
        outcome,
        reason,
        hopper: FoodLevel::Present,
        by,
    }
}

fn done(feed: u32, completed: u8, hopper: FoodLevel) -> EventBody {
    EventBody::Fed {
        feed,
        requested: completed,
        completed,
        outcome: FeedOutcome::Done,
        reason: FeedReason::Done,
        hopper,
        by: Source::Schedule(0),
    }
}

fn failed(feed: u32) -> EventBody {
    fed(
        feed,
        FeedOutcome::Failed,
        FeedReason::Jammed,
        Source::Schedule(0),
    )
}

fn boot(bank: u8, reset_flags: u32) -> EventBody {
    EventBody::Boot { bank, reset_flags }
}

const fn held(candidate: UpdateCandidate) -> EventBody {
    EventBody::UpdateAtBoot { candidate }
}

const fn stopped(stop: UpdateStop) -> EventBody {
    EventBody::UpdateStop { stop }
}

fn adapter(on: bool) -> EventBody {
    EventBody::Supply(Supply {
        adapter: Some(on),
        battery: Battery::Reading {
            mv: 5000,
            percent: 60,
        },
        below_floor: Some(false),
    })
}

fn joined(at: u32) -> (Option<u32>, EventBody) {
    (
        Some(at),
        EventBody::Enrolled {
            client: CLIENT,
            role: Role::Member,
            path: Path::Invite,
        },
    )
}

#[test]
fn an_entry_and_the_whole_state_stay_small() {
    assert_eq!(size_of::<OpenAlert>(), 16);
    assert_eq!(size_of::<[Option<OpenAlert>; CAPACITY]>(), 16 * CAPACITY);
    assert_eq!(size_of::<Alerts>(), 344);
}

#[test]
fn every_kind_has_its_level() {
    use AlertKind::*;

    for kind in [
        MealFailed,
        MealUncertain,
        MealMissed,
        MealNoRecord,
        NoClock,
        CannotFeed,
        FoodLikelyOut,
        PlansStuck,
    ] {
        assert_eq!(kind.level(), AlertLevel::Urgent, "{kind:?}");
    }
    for kind in [
        FoodLow,
        DesiccantDue,
        PowerWasOut,
        BatteryBelowFloor,
        UpdateFailed,
        RolledBack,
        RepeatedRestarts,
    ] {
        assert_eq!(kind.level(), AlertLevel::Soon, "{kind:?}");
    }
    for kind in [ClientJoined, ClientRevoked] {
        assert_eq!(kind.level(), AlertLevel::Security, "{kind:?}");
    }
}

// Reports: meals.

#[test]
fn a_feed_that_failed_or_expired_opens_with_its_reason_and_portions() {
    let mut alerts = settled();

    alerts.event(1, Some(WALL), &failed(4));
    let expired = fed(
        5,
        FeedOutcome::Expired,
        FeedReason::Output,
        Source::Client(CLIENT),
    );
    alerts.event(2, Some(WALL), &expired);

    let open: Vec<_> = alerts.open().copied().collect();
    assert_eq!(
        open[0].alert,
        Alert::MealFailed {
            feed: 4,
            plan: Some(0),
            requested: 2,
            completed: 1,
            outcome: FeedOutcome::Failed,
            reason: FeedReason::Jammed,
        }
    );
    assert_eq!(open[0].at.map(NonZeroU32::get), Some(WALL));
    assert_eq!(
        open[1].alert,
        Alert::MealFailed {
            feed: 5,
            plan: None,
            requested: 2,
            completed: 1,
            outcome: FeedOutcome::Expired,
            reason: FeedReason::Output,
        }
    );
}

#[test]
fn a_clean_or_cancelled_feed_opens_nothing() {
    let mut alerts = settled();

    alerts.event(1, Some(WALL), &done(4, 2, FoodLevel::Present));
    let cancelled = fed(
        5,
        FeedOutcome::Cancelled,
        FeedReason::Cancelled,
        Source::Panel,
    );
    alerts.event(2, Some(WALL), &cancelled);

    assert_eq!(kinds(&alerts), []);
}

#[test]
fn an_uncertain_feeds_second_fed_replaces_the_first_alert() {
    let mut alerts = settled();
    let by = Source::Schedule(0);

    alerts.event(
        1,
        Some(WALL),
        &fed(4, FeedOutcome::Uncertain, FeedReason::Output, by),
    );
    assert_eq!(kinds(&alerts), [AlertKind::MealUncertain]);

    alerts.event(
        2,
        None,
        &fed(4, FeedOutcome::Uncertain, FeedReason::Interrupted, by),
    );
    assert_eq!(kinds(&alerts), [AlertKind::MealUncertain]);
    let entry = alerts.open().next().unwrap();
    assert!(matches!(
        entry.alert,
        Alert::MealUncertain {
            reason: FeedReason::Interrupted,
            ..
        }
    ));

    alerts.event(3, Some(WALL), &failed(4));
    assert_eq!(kinds(&alerts), [AlertKind::MealFailed]);
}

#[test]
fn a_clean_second_fed_leaves_the_uncertain_alert() {
    let mut alerts = settled();
    let uncertain = fed(
        4,
        FeedOutcome::Uncertain,
        FeedReason::Interrupted,
        Source::Schedule(0),
    );

    alerts.event(1, Some(WALL), &uncertain);
    alerts.event(2, Some(WALL), &done(4, 2, FoodLevel::Present));

    assert_eq!(kinds(&alerts), [AlertKind::MealUncertain]);
}

#[test]
fn a_later_good_meal_does_not_undo_a_failed_one() {
    let mut alerts = settled();

    alerts.event(1, Some(WALL), &failed(4));
    alerts.event(2, Some(WALL), &done(5, 2, FoodLevel::Present));

    assert_eq!(kinds(&alerts), [AlertKind::MealFailed]);
}

#[test]
fn a_missed_meal_opens_once_with_its_reason() {
    let mut alerts = settled();
    let missed = EventBody::Missed {
        plan: 2,
        due: WALL,
        reason: MissedReason::Blocked(Blocked::Battery),
    };

    alerts.event(1, Some(WALL), &missed);
    alerts.event(2, Some(WALL), &missed);

    let open: Vec<_> = alerts.open().copied().collect();
    assert_eq!(open.len(), 1);
    assert_eq!(
        open[0].alert,
        Alert::MealMissed {
            plan: 2,
            due: WALL,
            reason: MissedReason::Blocked(Blocked::Battery),
        }
    );
}

#[test]
fn a_meal_with_no_record_opens_exactly_at_due_plus_its_grace() {
    let mut alerts = settled();
    let before = WALL - 1;

    alerts.meal(1, WALL + NO_RECORD_S - 1, 3, WALL, Some(before));
    assert_eq!(kinds(&alerts), []);

    alerts.meal(2, WALL + NO_RECORD_S, 3, WALL, Some(before));
    let open: Vec<_> = alerts.open().copied().collect();
    assert_eq!(open[0].alert, Alert::MealNoRecord { plan: 3, due: WALL });
    assert_eq!(open.len(), 1);
}

#[test]
fn a_meal_the_schedule_took_or_passed_has_its_record() {
    let mut alerts = settled();
    let late = WALL + NO_RECORD_S;

    alerts.meal(1, late, 3, WALL, Some(WALL));
    alerts.meal(2, late, 3, WALL, Some(WALL + 60));

    assert_eq!(kinds(&alerts), []);
}

/// A skip moves the plan's mark up to the meal it skips, which then reads as
/// one the schedule passed: its `Skipped` and `Unskipped` events open
/// nothing, and neither does the meal, however long after its time.
#[test]
fn a_skipped_meal_opens_nothing() {
    let mut alerts = settled();
    let by = Source::Client(CLIENT);

    alerts.event(
        1,
        Some(WALL - 3_600),
        &EventBody::Skipped {
            plan: 3,
            due: WALL,
            by,
        },
    );
    alerts.event(
        2,
        Some(WALL - 1_800),
        &EventBody::Unskipped {
            plan: 3,
            due: WALL,
            by,
        },
    );
    alerts.event(
        3,
        Some(WALL - 900),
        &EventBody::Skipped {
            plan: 3,
            due: WALL,
            by,
        },
    );
    alerts.meal(4, WALL + NO_RECORD_S, 3, WALL, Some(WALL));
    alerts.meal(5, WALL + 20 * HOUR_S, 3, WALL, Some(WALL));

    assert_eq!(kinds(&alerts), []);
}

#[test]
fn a_plan_with_no_mark_expects_no_first_occurrence() {
    let mut alerts = settled();

    alerts.meal(1, WALL + NO_RECORD_S, 3, WALL, None);

    assert_eq!(kinds(&alerts), []);
}

#[test]
fn a_record_that_arrives_late_ends_no_record() {
    let mut alerts = settled();
    let late = WALL + NO_RECORD_S;

    alerts.meal(1, late, 3, WALL, Some(WALL - 1));
    alerts.meal(2, late, 4, WALL, Some(WALL - 1));
    alerts.meal(3, late + 1, 3, WALL, Some(WALL));

    let open: Vec<_> = alerts.open().copied().collect();
    assert_eq!(open.len(), 1, "another plan's stays");
    assert_eq!(open[0].alert, Alert::MealNoRecord { plan: 4, due: WALL });
}

#[test]
fn a_missed_event_replaces_no_record_of_the_same_occurrence() {
    let mut alerts = settled();
    alerts.meal(1, WALL + NO_RECORD_S, 3, WALL, Some(WALL - 1));

    let missed = EventBody::Missed {
        plan: 3,
        due: WALL,
        reason: MissedReason::Late,
    };
    alerts.event(2, Some(WALL + NO_RECORD_S), &missed);

    assert_eq!(kinds(&alerts), [AlertKind::MealMissed]);
}

#[test]
fn a_no_record_pushed_out_does_not_come_straight_back() {
    let mut alerts = settled();
    let late = WALL + NO_RECORD_S;

    alerts.meal(1, late, 3, WALL, Some(WALL - 1));
    for feed in 0..CAPACITY {
        alerts.event(2, Some(late), &failed(u32::try_from(feed).unwrap()));
    }
    assert!(!kinds(&alerts).contains(&AlertKind::MealNoRecord));

    let generation = alerts.generation();
    alerts.meal(3, late + 60, 3, WALL, Some(WALL - 1));

    assert_eq!(alerts.generation(), generation);
    assert!(!kinds(&alerts).contains(&AlertKind::MealNoRecord));
}

#[test]
fn a_plan_out_of_range_is_ignored() {
    let mut alerts = settled();
    let plan = u8::try_from(PLANS).unwrap();

    alerts.meal(1, WALL + NO_RECORD_S, plan, WALL, Some(WALL - 1));

    assert_eq!(kinds(&alerts), []);
}

// Reports: the rest.

#[test]
fn power_out_opens_once_an_outage_and_again_for_the_next() {
    let mut alerts = settled();

    alerts.event(1, Some(WALL), &adapter(false));
    alerts.event(2, Some(WALL + 60), &adapter(false));
    assert_eq!(kinds(&alerts), [AlertKind::PowerWasOut]);

    alerts.event(3, Some(WALL + 120), &adapter(true));
    assert_eq!(
        kinds(&alerts),
        [AlertKind::PowerWasOut],
        "it stays a report"
    );

    alerts.event(4, Some(WALL + 180), &adapter(false));
    assert_eq!(
        kinds(&alerts),
        [AlertKind::PowerWasOut, AlertKind::PowerWasOut]
    );
}

#[test]
fn an_update_stop_opens_unless_the_owner_cancelled() {
    let mut alerts = settled();

    alerts.event(1, Some(WALL), &stopped(UpdateStop::Cancelled));
    assert_eq!(kinds(&alerts), []);

    alerts.event(2, Some(WALL), &stopped(UpdateStop::Hash));
    assert_eq!(
        alerts.open().next().unwrap().alert,
        Alert::UpdateFailed {
            stop: UpdateStop::Hash
        }
    );
}

#[test]
fn a_client_joining_and_a_revoke_open_and_a_rejoin_alerts_again() {
    let mut alerts = settled();

    let (at, body) = joined(WALL);
    alerts.event(1, at, &body);
    alerts.event(1, at, &body);
    assert_eq!(kinds(&alerts), [AlertKind::ClientJoined]);

    let revoked = EventBody::Revoked {
        client: CLIENT,
        by: Source::Panel,
    };
    alerts.event(2, Some(WALL + 1), &revoked);

    let (at, body) = joined(WALL + 2);
    alerts.event(3, at, &body);
    assert_eq!(
        kinds(&alerts),
        [
            AlertKind::ClientJoined,
            AlertKind::ClientRevoked,
            AlertKind::ClientJoined
        ]
    );
}

/// What the table says once a pass has looked at it.
fn after_a_pass(alerts: &mut Alerts) -> Vec<AlertKind> {
    alerts.step(1, &calm());
    kinds(alerts)
}

#[test]
fn repeated_restarts_open_at_three_in_the_window() {
    let mut alerts = settled();

    alerts.event(0, Some(WALL), &boot(1, WATCHDOG));
    alerts.event(0, Some(WALL), &held(UpdateCandidate::None));
    alerts.event(0, Some(WALL + HOUR_S), &boot(1, WATCHDOG));
    alerts.event(0, Some(WALL + HOUR_S), &held(UpdateCandidate::None));
    assert_eq!(after_a_pass(&mut alerts), []);

    alerts.event(0, Some(WALL + RESTART_WINDOW_S), &boot(1, WATCHDOG));
    alerts.event(
        0,
        Some(WALL + RESTART_WINDOW_S),
        &held(UpdateCandidate::None),
    );
    assert_eq!(after_a_pass(&mut alerts), [AlertKind::RepeatedRestarts]);
}

#[test]
fn restarts_spread_out_or_planned_do_not_count() {
    let mut alerts = settled();

    alerts.event(0, Some(WALL), &boot(1, WATCHDOG));
    alerts.event(0, Some(WALL), &held(UpdateCandidate::None));
    alerts.event(0, Some(WALL + HOUR_S), &boot(1, WATCHDOG));
    alerts.event(0, Some(WALL + HOUR_S), &held(UpdateCandidate::None));
    alerts.event(0, Some(WALL + RESTART_WINDOW_S + 1), &boot(1, WATCHDOG));
    alerts.event(
        0,
        Some(WALL + RESTART_WINDOW_S + 1),
        &held(UpdateCandidate::None),
    );
    assert_eq!(after_a_pass(&mut alerts), [], "over the window");

    let mut alerts = settled();
    for _ in 0..3 {
        alerts.event(0, Some(WALL), &boot(1, 0));
        alerts.event(0, Some(WALL), &held(UpdateCandidate::None));
    }
    assert_eq!(after_a_pass(&mut alerts), [], "power-ons");
}

#[test]
fn a_boot_with_no_time_counts_as_recent() {
    let mut alerts = settled();

    for _ in 0..3 {
        alerts.event(0, None, &boot(1, WATCHDOG));
        alerts.event(0, None, &held(UpdateCandidate::None));
    }

    assert_eq!(after_a_pass(&mut alerts), [AlertKind::RepeatedRestarts]);
}

#[test]
fn an_episode_of_restarts_ends_after_a_quiet_day_and_a_later_one_alerts_again() {
    let mut alerts = settled();
    for _ in 0..3 {
        alerts.event(0, None, &boot(1, WATCHDOG));
        alerts.event(0, None, &held(UpdateCandidate::None));
    }

    alerts.step(RESTARTS_QUIET_MS - 1, &calm());
    assert_eq!(kinds(&alerts), [AlertKind::RepeatedRestarts]);

    alerts.step(RESTARTS_QUIET_MS, &calm());
    assert_eq!(kinds(&alerts), []);

    for _ in 0..2 {
        alerts.event(0, None, &boot(1, WATCHDOG));
        alerts.event(0, None, &held(UpdateCandidate::None));
    }
    assert_eq!(after_a_pass(&mut alerts), [], "the count started again");
    alerts.event(0, None, &boot(1, WATCHDOG));
    alerts.event(0, None, &held(UpdateCandidate::None));
    assert_eq!(after_a_pass(&mut alerts), [AlertKind::RepeatedRestarts]);
}

#[test]
fn restarts_read_at_boot_from_long_ago_end_once_the_clock_says_so() {
    let mut alerts = Alerts::new();
    for _ in 0..3 {
        alerts.replay(0, None, Some(WALL), &boot(1, WATCHDOG));
        alerts.replay(0, None, Some(WALL), &held(UpdateCandidate::None));
    }
    alerts.restored(0, History::Whole);
    let no_clock = Readings {
        wall_s: None,
        ..calm()
    };
    alerts.step(1, &no_clock);
    assert_eq!(kinds(&alerts), [AlertKind::RepeatedRestarts]);

    let later = Readings {
        wall_s: Some(WALL + RESTARTS_QUIET_S),
        ..calm()
    };
    alerts.step(1000, &later);

    assert_eq!(kinds(&alerts), []);
}

// The boot read.

#[test]
fn old_reports_in_the_ring_neither_open_nor_count_as_room_running_out() {
    let mut alerts = Alerts::new();

    // Far more old reports than fit, a day apart, then a few recent ones.
    for day in 0..40 {
        let at = WALL + day * 24 * HOUR_S;
        alerts.replay(0, None, Some(at), &failed(day));
    }
    let recent = WALL + 40 * 24 * HOUR_S;
    alerts.replay(0, None, Some(recent), &failed(100));
    alerts.replay(0, None, Some(recent + 1), &failed(101));
    alerts.restored(0, History::Whole);

    assert!(!alerts.more());
    assert_eq!(feeds(&alerts), [39, 100, 101]);

    // The one a day older than the newest is still within its life.
    let later = Readings {
        wall_s: Some(recent + REPORT_LIFE_S),
        ..calm()
    };
    alerts.step(1, &later);
    assert_eq!(feeds(&alerts), [101]);
}

#[test]
fn a_report_older_than_its_life_against_the_newest_seen_stays_out() {
    let mut alerts = Alerts::new();

    alerts.replay(0, None, Some(WALL), &done(1, 1, FoodLevel::Present));
    alerts.replay(0, None, Some(WALL - REPORT_LIFE_S), &failed(4));
    alerts.replay(0, None, Some(WALL - REPORT_LIFE_S + 1), &failed(5));
    alerts.restored(0, History::Whole);

    assert_eq!(feeds(&alerts), [5]);
}

#[test]
fn a_report_older_than_the_newest_time_any_event_shows_lapses_when_the_read_ends() {
    let mut alerts = Alerts::new();

    alerts.replay(0, None, Some(WALL), &failed(4));
    alerts.replay(
        0,
        None,
        Some(WALL + REPORT_LIFE_S),
        &done(5, 1, FoodLevel::Present),
    );
    assert_eq!(feeds(&alerts), [4]);

    alerts.restored(0, History::Whole);
    assert_eq!(feeds(&alerts), []);
    assert!(!alerts.more());
}

#[test]
fn recent_reports_that_truly_do_not_fit_still_set_more() {
    let mut alerts = Alerts::new();

    for feed in 0..13 {
        alerts.replay(0, None, Some(WALL + feed), &failed(feed));
    }
    alerts.restored(0, History::Whole);

    assert!(alerts.more());
    assert_eq!(feeds(&alerts).len(), CAPACITY);
}

#[test]
fn a_live_report_with_an_old_time_still_opens_after_the_read() {
    let mut alerts = settled();

    // One bad time ahead must not silence what follows.
    alerts.event(1, Some(WALL + 10 * REPORT_LIFE_S), &failed(4));
    alerts.event(2, Some(WALL), &failed(5));

    assert_eq!(feeds(&alerts), [4, 5]);
}

#[test]
fn an_episode_the_times_show_is_quiet_does_not_open_at_boot() {
    let mut alerts = Alerts::new();

    for _ in 0..3 {
        alerts.replay(0, None, Some(WALL), &boot(1, WATCHDOG));
        alerts.replay(0, None, Some(WALL), &held(UpdateCandidate::None));
    }
    alerts.replay(
        0,
        None,
        Some(WALL + RESTARTS_QUIET_S),
        &done(1, 1, FoodLevel::Present),
    );
    alerts.restored(0, History::Whole);

    assert_eq!(after_a_pass(&mut alerts), []);
}

#[test]
fn an_old_episode_does_not_join_this_boots_restart() {
    let mut alerts = Alerts::new();

    alerts.replay(0, None, Some(WALL), &boot(1, WATCHDOG));
    alerts.replay(0, None, Some(WALL), &held(UpdateCandidate::None));
    alerts.replay(0, None, Some(WALL + 60), &boot(1, WATCHDOG));
    alerts.replay(0, None, Some(WALL + 60), &held(UpdateCandidate::None));
    alerts.replay(
        0,
        None,
        Some(WALL + RESTARTS_QUIET_S + 60),
        &done(1, 1, FoodLevel::Present),
    );
    alerts.event(0, None, &boot(1, WATCHDOG));
    alerts.event(0, None, &held(UpdateCandidate::None));
    alerts.restored(0, History::Whole);

    assert_eq!(after_a_pass(&mut alerts), []);
}

#[test]
fn an_episode_still_recent_in_the_ring_opens_at_boot() {
    let mut alerts = Alerts::new();

    for _ in 0..3 {
        alerts.replay(0, None, Some(WALL), &boot(1, WATCHDOG));
        alerts.replay(0, None, Some(WALL), &held(UpdateCandidate::None));
    }
    alerts.replay(
        0,
        None,
        Some(WALL + RESTARTS_QUIET_S - 1),
        &done(1, 1, FoodLevel::Present),
    );
    alerts.restored(0, History::Whole);

    assert_eq!(after_a_pass(&mut alerts), [AlertKind::RepeatedRestarts]);
}

#[test]
fn an_episode_with_no_times_stands_as_a_false_alert_we_accept() {
    let mut alerts = Alerts::new();

    for _ in 0..3 {
        alerts.replay(0, None, None, &boot(1, WATCHDOG));
        alerts.replay(0, None, None, &held(UpdateCandidate::None));
    }
    alerts.replay(0, None, Some(WALL), &done(1, 1, FoodLevel::Present));
    alerts.restored(0, History::Whole);

    assert_eq!(after_a_pass(&mut alerts), [AlertKind::RepeatedRestarts]);
}

#[test]
fn no_repeated_restarts_open_before_the_read_ends() {
    let mut alerts = Alerts::new();

    for _ in 0..3 {
        alerts.replay(0, None, None, &boot(1, WATCHDOG));
        alerts.replay(0, None, None, &held(UpdateCandidate::None));
    }
    assert_eq!(after_a_pass(&mut alerts), []);
}

#[test]
fn this_boots_boot_counts_once_and_after_the_boots_the_read_finds() {
    // Reaching the recorder first, it still counts as the newest, once.
    let mut alerts = Alerts::new();
    alerts.event(0, None, &boot(0, WATCHDOG));
    alerts.event(0, None, &held(UpdateCandidate::None));
    alerts.replay(0, None, None, &boot(1, WATCHDOG));
    alerts.replay(0, None, None, &held(UpdateCandidate::None));
    alerts.replay(0, None, None, &boot(1, WATCHDOG));
    alerts.replay(0, None, None, &held(UpdateCandidate::None));
    assert_eq!(alerts.restarts, 2, "it waits for the read");
    assert_eq!(kinds(&alerts), []);

    alerts.restored(0, History::Whole);
    assert_eq!(alerts.restarts, 3);
    assert_eq!(
        kinds(&alerts),
        [AlertKind::RolledBack],
        "bank 1, then bank 0"
    );

    alerts.restored(0, History::Whole);
    assert_eq!(alerts.restarts, 3, "a second end of the read adds nothing");

    alerts.event(0, None, &boot(0, WATCHDOG));
    alerts.event(0, None, &held(UpdateCandidate::None));
    assert_eq!(alerts.restarts, 4, "a later boot is a boot");
}

// How a boot got there.

/// One boot and its update record, as the recorder sees them.
fn booted(alerts: &mut Alerts, at: u32, bank: u8, flags: u32, candidate: UpdateCandidate) {
    alerts.event(0, Some(at), &boot(bank, flags));
    alerts.event(0, Some(at), &held(candidate));
}

#[test]
fn a_fall_from_bank_1_with_nothing_pending_is_a_rollback_at_once() {
    let mut alerts = settled();
    booted(&mut alerts, WALL, 1, 0, UpdateCandidate::None);
    booted(&mut alerts, WALL + 60, 0, WATCHDOG, UpdateCandidate::None);

    let entry = alerts.open().next().unwrap();
    assert_eq!(entry.alert, Alert::RolledBack);
    assert_eq!(entry.at, NonZeroU32::new(WALL + 60));
}

#[test]
fn a_trial_that_gave_way_finds_its_candidate_activated_and_alerts_once() {
    let mut alerts = settled();
    booted(&mut alerts, WALL, 0, 0, UpdateCandidate::Pending);
    booted(
        &mut alerts,
        WALL + 60,
        1,
        WATCHDOG,
        UpdateCandidate::Activated,
    );
    booted(
        &mut alerts,
        WALL + 120,
        1,
        WATCHDOG,
        UpdateCandidate::Activated,
    );
    assert_eq!(kinds(&alerts), [], "still on trial");

    booted(
        &mut alerts,
        WALL + 180,
        0,
        WATCHDOG,
        UpdateCandidate::Activated,
    );
    assert_eq!(kinds(&alerts), [AlertKind::RolledBack]);

    booted(
        &mut alerts,
        WALL + 240,
        0,
        WATCHDOG,
        UpdateCandidate::Activated,
    );
    assert_eq!(alerts.open().count(), 1, "bank 0 goes on finding it");
}

#[test]
fn an_activation_that_never_reached_bank_1_is_a_rollback_too() {
    let mut alerts = settled();
    booted(&mut alerts, WALL, 0, 0, UpdateCandidate::Pending);
    booted(
        &mut alerts,
        WALL + 60,
        0,
        WATCHDOG,
        UpdateCandidate::Activated,
    );

    assert_eq!(kinds(&alerts), [AlertKind::RolledBack]);
}

/// Time plays no part: this install takes a day, and a clock and uptime well
/// past any wait still find nothing wrong.
#[test]
fn a_slow_good_install_is_no_rollback_and_no_restarts() {
    let mut alerts = settled();
    let mut at = WALL;
    for _ in 0..3 {
        booted(&mut alerts, at, 1, 0, UpdateCandidate::None);
        at += 60;
        booted(&mut alerts, at, 0, WATCHDOG, UpdateCandidate::Pending);
        alerts.step(
            HOUR_MS,
            &Readings {
                wall_s: Some(at),
                ..calm()
            },
        );
        assert_eq!(kinds(&alerts), []);

        at += 60;
        booted(&mut alerts, at, 1, WATCHDOG, UpdateCandidate::Activated);
        at += 60;
    }

    alerts.step(24 * HOUR_MS, &calm());
    assert_eq!(kinds(&alerts), []);
    assert_eq!(alerts.restarts, 0);
}

#[test]
fn a_later_stop_leaves_a_real_rollback_alone() {
    let mut alerts = settled();
    booted(&mut alerts, WALL, 1, 0, UpdateCandidate::None);
    booted(&mut alerts, WALL + 60, 0, 0, UpdateCandidate::None);
    assert_eq!(kinds(&alerts), [AlertKind::RolledBack]);

    alerts.event(1, Some(WALL + 120), &stopped(UpdateStop::Cancelled));
    assert_eq!(kinds(&alerts), [AlertKind::RolledBack]);

    alerts.event(2, Some(WALL + 180), &stopped(UpdateStop::Timeout));
    assert_eq!(
        kinds(&alerts),
        [AlertKind::RolledBack, AlertKind::UpdateFailed]
    );
}

#[test]
fn a_feeder_first_seen_on_bank_0_fell_from_nowhere() {
    for candidate in [UpdateCandidate::None, UpdateCandidate::Activated] {
        let mut alerts = settled();
        booted(&mut alerts, WALL, 0, 0, candidate);
        assert_eq!(kinds(&alerts), [], "{candidate:?}");
    }
}

/// A reset can beat the record to the ring. The next boot's record says what
/// the store holds, which outlasted both.
#[test]
fn a_boot_that_never_got_its_record_is_judged_by_the_next_one() {
    let mut alerts = settled();
    booted(&mut alerts, WALL, 1, 0, UpdateCandidate::None);
    alerts.event(0, Some(WALL + 60), &boot(0, WATCHDOG));
    assert_eq!(kinds(&alerts), [], "no record, no verdict");
    assert_eq!(alerts.restarts, 0);

    booted(&mut alerts, WALL + 120, 0, WATCHDOG, UpdateCandidate::None);
    assert_eq!(kinds(&alerts), [AlertKind::RolledBack]);
    assert_eq!(alerts.restarts, 2, "with no record it counts by its flags");
}

#[test]
fn a_record_with_no_boot_before_it_judges_nothing() {
    let mut alerts = settled();
    alerts.event(0, Some(WALL), &held(UpdateCandidate::Activated));
    assert_eq!(kinds(&alerts), []);
    assert_eq!(alerts.judged, None);
}

#[test]
fn a_reset_in_the_middle_of_an_install_counts() {
    let mut alerts = settled();
    booted(&mut alerts, WALL, 0, 0, UpdateCandidate::Pending);
    booted(
        &mut alerts,
        WALL + 60,
        0,
        WATCHDOG,
        UpdateCandidate::Pending,
    );
    assert_eq!(alerts.restarts, 1);
}

#[test]
fn a_restart_someone_asked_for_does_not_count_and_only_the_next_boot_is_excused() {
    let mut alerts = settled();
    booted(&mut alerts, WALL, 1, 0, UpdateCandidate::None);
    alerts.event(0, Some(WALL + 1), &EventBody::Restarting);
    booted(&mut alerts, WALL + 60, 1, WATCHDOG, UpdateCandidate::None);
    assert_eq!(alerts.restarts, 0);

    booted(&mut alerts, WALL + 120, 1, WATCHDOG, UpdateCandidate::None);
    assert_eq!(alerts.restarts, 1);
}

/// A restart asked for before the boot read ends is this boot's own: it
/// excuses the boot after it, never the one that recorded it.
#[test]
fn a_restart_asked_for_during_the_boot_read_excuses_only_the_next_boot() {
    let mut alerts = Alerts::new();
    alerts.event(0, Some(WALL), &boot(1, WATCHDOG));
    alerts.event(0, Some(WALL), &held(UpdateCandidate::None));
    alerts.event(0, Some(WALL + 1), &EventBody::Restarting);
    alerts.restored(0, History::Whole);
    assert_eq!(alerts.restarts, 1, "this boot counts by its flags");

    booted(&mut alerts, WALL + 60, 1, WATCHDOG, UpdateCandidate::None);
    assert_eq!(alerts.restarts, 1, "the boot it asked for is excused");
}

// Bank 0 keeps the build it was written with: one from before the update
// record writes a `Boot` and nothing more.

/// An older bank 0's boot with no record of its own.
fn old_bank_0(alerts: &mut Alerts, at: u32) {
    alerts.event(0, Some(at), &boot(0, WATCHDOG));
}

#[test]
fn an_install_through_an_older_bank_0_is_no_restart() {
    let mut alerts = settled();
    booted(&mut alerts, WALL, 1, 0, UpdateCandidate::None);

    // Two installs and a fault within the window: only the fault counts.
    for round in 0..2 {
        let at = WALL + round * 600;
        old_bank_0(&mut alerts, at + 60);
        booted(
            &mut alerts,
            at + 120,
            1,
            WATCHDOG,
            UpdateCandidate::Activated,
        );
        booted(&mut alerts, at + 180, 1, 0, UpdateCandidate::None);
    }
    booted(&mut alerts, WALL + 1300, 1, WATCHDOG, UpdateCandidate::None);

    assert_eq!(alerts.restarts, 1);
    assert_eq!(kinds(&alerts), []);
}

#[test]
fn a_fall_to_an_older_bank_0_after_an_activation_is_a_rollback() {
    let mut alerts = settled();
    booted(&mut alerts, WALL, 1, 0, UpdateCandidate::None);
    old_bank_0(&mut alerts, WALL + 60);
    booted(
        &mut alerts,
        WALL + 120,
        1,
        WATCHDOG,
        UpdateCandidate::Activated,
    );

    // The build never confirms, and bank 0 runs a while.
    old_bank_0(&mut alerts, WALL + 180);
    old_bank_0(&mut alerts, WALL + 240);
    assert_eq!(kinds(&alerts), [], "bank 0 may yet record");

    // An install from bank 0 brings bank 1 back.
    booted(
        &mut alerts,
        WALL + 300,
        1,
        WATCHDOG,
        UpdateCandidate::Activated,
    );
    let rolled: Vec<_> = alerts
        .open()
        .filter(|open| open.alert == Alert::RolledBack)
        .collect();
    assert_eq!(rolled.len(), 1, "{:?}", kinds(&alerts));
    assert_eq!(rolled[0].at, NonZeroU32::new(WALL + 180));
    assert_eq!(alerts.restarts, 0);
}

#[test]
fn reading_the_same_boots_back_gives_the_same_verdicts() {
    let boots = [
        (WALL, 1, 0, UpdateCandidate::None),
        (WALL + 60, 0, WATCHDOG, UpdateCandidate::Pending),
        (WALL + 120, 1, WATCHDOG, UpdateCandidate::Activated),
        (WALL + 180, 0, WATCHDOG, UpdateCandidate::Activated),
        (WALL + 240, 0, WATCHDOG, UpdateCandidate::None),
        (WALL + 300, 1, WATCHDOG, UpdateCandidate::None),
        (WALL + 360, 0, WATCHDOG, UpdateCandidate::None),
    ];

    let mut live = settled();
    let mut read_back = Alerts::new();
    for (at, bank, flags, candidate) in boots {
        booted(&mut live, at, bank, flags, candidate);
        read_back.replay(0, None, Some(at), &boot(bank, flags));
        read_back.replay(0, None, Some(at), &held(candidate));
    }
    read_back.restored(0, History::Whole);

    assert_eq!(live.table, read_back.table);
    assert_eq!(live.restarts, read_back.restarts);
}

// A read that fell short.

#[test]
fn a_read_that_gave_up_keeps_more_for_a_whole_report_life() {
    let mut alerts = Alerts::new();
    alerts.restored(10_000, History::Unread);
    assert!(alerts.more());

    alerts.step(10_000 + MORE_MS - 1, &calm());
    assert!(alerts.more());
    alerts.step(10_000 + MORE_MS, &calm());
    assert!(!alerts.more());
}

#[test]
fn a_whole_read_sets_nothing() {
    let mut alerts = Alerts::new();
    alerts.restored(10_000, History::Whole);
    assert!(!alerts.more());
}

#[test]
fn a_read_cut_short_counts_from_the_oldest_time_it_found() {
    // Everything it missed is older than an hour before the newest time.
    let mut alerts = Alerts::new();
    alerts.replay(0, None, Some(WALL), &failed(1));
    alerts.replay(0, None, Some(WALL + HOUR_S), &failed(2));
    alerts.restored(
        1_000,
        History::Cut {
            oldest_s: Some(WALL),
        },
    );
    assert!(alerts.more());

    alerts.step(
        1_000 + MORE_MS - HOUR_MS - 1,
        &Readings {
            wall_s: None,
            ..calm()
        },
    );
    assert!(alerts.more());
    alerts.step(
        1_000 + MORE_MS - HOUR_MS,
        &Readings {
            wall_s: None,
            ..calm()
        },
    );
    assert!(!alerts.more());
}

#[test]
fn a_read_cut_short_past_every_report_life_cannot_have_missed_anything() {
    let mut alerts = Alerts::new();
    alerts.replay(0, None, Some(WALL), &boot(1, 0));
    alerts.replay(0, None, Some(WALL + REPORT_LIFE_S), &boot(1, 0));
    alerts.restored(
        1_000,
        History::Cut {
            oldest_s: Some(WALL),
        },
    );
    assert!(!alerts.more());
}

#[test]
fn a_read_cut_short_with_no_times_keeps_more_for_a_whole_report_life() {
    let mut alerts = Alerts::new();
    alerts.restored(1_000, History::Cut { oldest_s: None });
    assert!(alerts.more());
}

#[test]
fn the_welcome_does_not_wait_for_a_read_past_its_bound() {
    let mut alerts = Alerts::new();
    assert!(!alerts.welcoming());
    alerts.welcome();
    assert!(alerts.welcoming());
    assert!(!alerts.is_restored());

    let mut alerts = Alerts::new();
    alerts.restored(0, History::Whole);
    assert!(alerts.welcoming());
}

// What the read finds against what this boot already said.

#[test]
fn a_read_never_replaces_a_report_with_an_older_time() {
    let uncertain = fed(
        1,
        FeedOutcome::Uncertain,
        FeedReason::Interrupted,
        Source::Schedule(0),
    );

    let mut alerts = Alerts::new();
    alerts.event(0, Some(WALL + 60), &failed(1));
    alerts.replay(0, None, Some(WALL), &uncertain);
    let open = alerts.open().next().unwrap();
    assert_eq!(open.at, NonZeroU32::new(WALL + 60));
    assert!(matches!(open.alert, Alert::MealFailed { .. }));

    // In its own order, the newer still wins.
    let mut alerts = Alerts::new();
    alerts.replay(0, None, Some(WALL), &uncertain);
    alerts.replay(0, None, Some(WALL + 60), &failed(1));
    assert!(matches!(
        alerts.open().next().unwrap().alert,
        Alert::MealFailed { .. }
    ));

    // Once it is read, a time is whatever the clock says.
    let mut alerts = settled();
    alerts.event(0, Some(WALL + 60), &failed(1));
    alerts.event(0, Some(WALL), &uncertain);
    assert!(matches!(
        alerts.open().next().unwrap().alert,
        Alert::MealUncertain { .. }
    ));
}

#[test]
fn a_read_of_the_supply_changes_nothing_this_boot_reported() {
    // The adapter is back this boot, and the ring still ends with it out.
    let mut alerts = Alerts::new();
    alerts.event(0, Some(WALL + 60), &adapter(true));
    alerts.replay(0, None, Some(WALL), &adapter(false));
    alerts.restored(0, History::Whole);
    assert_eq!(kinds(&alerts), []);
    assert!(!alerts.power_out);

    // Out this boot, and in on the ring.
    let mut alerts = Alerts::new();
    alerts.event(0, Some(WALL + 60), &adapter(false));
    alerts.replay(0, None, Some(WALL), &adapter(true));
    alerts.restored(0, History::Whole);
    assert_eq!(kinds(&alerts), [AlertKind::PowerWasOut]);
}

#[test]
fn a_time_that_cannot_be_right_says_nothing_of_how_far_on_the_ring_got() {
    // Ahead of the clock, by far: everything real would lapse against it.
    let mut alerts = Alerts::new();
    alerts.replay(0, Some(WALL), Some(WALL + 10), &failed(1));
    alerts.replay(0, Some(WALL), Some(WALL + 400 * DAY_S), &failed(2));
    alerts.replay(0, Some(WALL), Some(WALL + 20), &failed(3));
    alerts.restored(0, History::Whole);
    assert_eq!(alerts.newest_s, WALL + 20);
    assert_eq!(feeds(&alerts), [1, 2, 3]);

    // With no clock, the jump gives it away.
    let mut alerts = Alerts::new();
    alerts.replay(0, None, Some(WALL), &failed(1));
    alerts.replay(0, None, Some(WALL + 2 * JUMP_AHEAD_S), &failed(2));
    assert_eq!(alerts.newest_s, WALL);

    // Well within it, a feeder that was off for a while is just that.
    let mut alerts = Alerts::new();
    alerts.replay(0, None, Some(WALL), &failed(1));
    alerts.replay(0, None, Some(WALL + 30 * DAY_S), &failed(2));
    alerts.restored(0, History::Whole);
    assert_eq!(feeds(&alerts), [2]);
}

// Reports lapse.

#[test]
fn a_report_lapses_48_hours_after_its_own_time() {
    let mut alerts = settled();
    alerts.event(1, Some(WALL), &failed(4));

    let at = |wall_s| Readings {
        wall_s: Some(wall_s),
        ..calm()
    };
    alerts.step(2, &at(WALL + REPORT_LIFE_S - 1));
    assert_eq!(kinds(&alerts), [AlertKind::MealFailed]);

    alerts.step(3, &at(WALL + REPORT_LIFE_S));
    assert_eq!(kinds(&alerts), []);
}

#[test]
fn a_report_with_no_time_stays_until_room_runs_out() {
    let mut alerts = settled();
    alerts.event(1, None, &failed(4));

    let far = Readings {
        wall_s: Some(WALL + 10 * REPORT_LIFE_S),
        ..calm()
    };
    alerts.step(2, &far);
    assert_eq!(kinds(&alerts), [AlertKind::MealFailed]);
}

#[test]
fn a_clock_stepping_back_lapses_nothing() {
    let mut alerts = settled();
    alerts.event(1, Some(WALL), &failed(4));

    let back = Readings {
        wall_s: Some(WALL - REPORT_LIFE_S * 2),
        ..calm()
    };
    alerts.step(2, &back);

    assert_eq!(kinds(&alerts), [AlertKind::MealFailed]);
}

#[test]
fn a_condition_never_lapses() {
    let mut alerts = settled();
    let low = Readings {
        food: Some(false),
        ..calm()
    };
    alerts.step(1, &low);

    let far = Readings {
        wall_s: Some(WALL + 10 * REPORT_LIFE_S),
        ..low
    };
    alerts.step(2, &far);

    assert_eq!(kinds(&alerts), [AlertKind::FoodLow]);
}

// Conditions.

#[test]
fn no_clock_opens_past_its_threshold_and_ends_with_the_state() {
    let mut alerts = settled();
    let no_clock = Readings {
        wall_s: None,
        ..calm()
    };

    alerts.step(1000, &no_clock);
    alerts.step(1000 + NO_CLOCK_MS - 1, &no_clock);
    assert_eq!(kinds(&alerts), []);

    alerts.step(1000 + NO_CLOCK_MS, &no_clock);
    assert_eq!(kinds(&alerts), [AlertKind::NoClock]);

    alerts.step(1001 + NO_CLOCK_MS, &calm());
    assert_eq!(kinds(&alerts), []);
}

/// NTP sets the clock after a cold boot but carries no offset, and a
/// client's sync lived in RAM: with no zone rule, every plan holds. That is
/// no clock to the meals, and says so after the same hour.
#[test]
fn a_valid_clock_with_no_zone_offset_opens_no_clock() {
    let mut alerts = settled();
    let unzoned = Readings {
        zoned: false,
        plans_settled: false,
        ..calm()
    };

    alerts.step(1000, &unzoned);
    alerts.step(1000 + NO_CLOCK_MS - 1, &unzoned);
    assert_eq!(kinds(&alerts), []);

    // Stuck plans need local time too, so the two never open together.
    alerts.step(1000 + NO_CLOCK_MS, &unzoned);
    assert_eq!(kinds(&alerts), [AlertKind::NoClock]);

    alerts.step(1001 + NO_CLOCK_MS, &calm());
    assert_eq!(kinds(&alerts), []);
}

#[test]
fn a_condition_past_its_threshold_survives_the_uptime_wrapping() {
    let mut alerts = settled();
    let no_clock = Readings {
        wall_s: None,
        ..calm()
    };

    alerts.step(0, &no_clock);
    alerts.step(NO_CLOCK_MS, &no_clock);
    alerts.step(NO_CLOCK_MS / 2, &no_clock);

    assert_eq!(kinds(&alerts), [AlertKind::NoClock]);
}

#[test]
fn a_timer_counts_across_the_uptime_wrapping() {
    let mut alerts = settled();
    let no_clock = Readings {
        wall_s: None,
        ..calm()
    };
    let start = u32::MAX - 1000;

    alerts.step(start, &no_clock);
    alerts.step(start.wrapping_add(NO_CLOCK_MS - 1), &no_clock);
    assert_eq!(kinds(&alerts), []);

    alerts.step(start.wrapping_add(NO_CLOCK_MS), &no_clock);
    assert_eq!(kinds(&alerts), [AlertKind::NoClock]);
}

#[test]
fn cannot_feed_opens_past_its_threshold_with_the_reason_and_ends_on_ready() {
    let mut alerts = settled();
    let blocked = |reason| Readings {
        blocked: Some(reason),
        ..calm()
    };

    alerts.step(0, &blocked(Blocked::Joining));
    alerts.step(CANNOT_FEED_MS - 1, &blocked(Blocked::Joining));
    assert_eq!(kinds(&alerts), []);

    alerts.step(CANNOT_FEED_MS, &blocked(Blocked::Joining));
    assert_eq!(
        alerts.open().next().unwrap().alert,
        Alert::CannotFeed {
            reason: Blocked::Joining
        }
    );

    let at = alerts.open().next().unwrap().at;
    let generation = alerts.generation();
    alerts.step(CANNOT_FEED_MS + 1, &blocked(Blocked::Fault));
    let entry = alerts.open().next().unwrap();
    assert_eq!(
        entry.alert,
        Alert::CannotFeed {
            reason: Blocked::Fault
        }
    );
    assert_eq!(entry.at, at);
    assert_ne!(alerts.generation(), generation);

    alerts.step(CANNOT_FEED_MS + 2, &calm());
    assert_eq!(kinds(&alerts), []);
}

#[test]
fn a_stuck_claim_says_why_it_blocks_past_its_wait_and_ends_with_the_claim() {
    let mut alerts = settled();
    let claim = Readings {
        blocked: Some(Blocked::Feeding),
        claim: true,
        ..calm()
    };

    alerts.step(1_000, &claim);
    alerts.step(1_000 + CLAIM_WAIT_MS - 1, &claim);
    assert_eq!(
        kinds(&alerts),
        [],
        "a claim takes seconds, and can wait a turn"
    );

    alerts.step(1_000 + CLAIM_WAIT_MS, &claim);
    assert_eq!(
        alerts.open().next().unwrap().alert,
        Alert::CannotFeed {
            reason: Blocked::Feeding
        }
    );

    alerts.step(1_000 + CLAIM_WAIT_MS + 1, &calm());
    assert_eq!(kinds(&alerts), []);
}

#[test]
fn plans_out_of_reach_alert_past_their_wait_only_with_a_clock() {
    let stuck = Readings {
        plans_settled: false,
        ..calm()
    };

    let mut alerts = settled();
    alerts.step(1_000, &stuck);
    alerts.step(1_000 + PLANS_STUCK_MS - 1, &stuck);
    assert_eq!(kinds(&alerts), []);

    alerts.step(1_000 + PLANS_STUCK_MS, &stuck);
    assert_eq!(kinds(&alerts), [AlertKind::PlansStuck]);

    alerts.step(1_000 + PLANS_STUCK_MS + 1, &calm());
    assert_eq!(kinds(&alerts), [], "a change that finished ends it");

    let no_clock = Readings {
        wall_s: None,
        ..stuck
    };
    let mut alerts = settled();
    alerts.step(1_000, &no_clock);
    alerts.step(1_000 + PLANS_STUCK_MS, &no_clock);
    assert!(!kinds(&alerts).contains(&AlertKind::PlansStuck));
}

#[test]
fn a_ready_moment_starts_cannot_feed_over() {
    let mut alerts = settled();
    let blocked = Readings {
        blocked: Some(Blocked::Feeding),
        ..calm()
    };

    alerts.step(0, &blocked);
    alerts.step(CANNOT_FEED_MS - 1, &calm());
    alerts.step(CANNOT_FEED_MS, &blocked);
    alerts.step(2 * CANNOT_FEED_MS - 1, &blocked);

    assert_eq!(kinds(&alerts), []);
}

#[test]
fn food_low_follows_the_beam() {
    let mut alerts = settled();
    let low = Readings {
        food: Some(false),
        ..calm()
    };

    alerts.step(1, &low);
    assert_eq!(kinds(&alerts), [AlertKind::FoodLow]);

    alerts.step(2, &calm());
    assert_eq!(kinds(&alerts), []);
}

#[test]
fn food_is_likely_out_after_five_portions_past_low_and_a_refill_ends_it() {
    let mut alerts = settled();
    let low = Readings {
        food: Some(false),
        ..calm()
    };

    alerts.step(1, &low);
    alerts.event(2, Some(WALL), &done(1, 4, FoodLevel::Low));
    alerts.step(3, &low);
    assert_eq!(kinds(&alerts), [AlertKind::FoodLow]);

    alerts.event(4, Some(WALL), &done(2, 1, FoodLevel::Low));
    alerts.step(5, &low);
    assert_eq!(
        kinds(&alerts),
        [AlertKind::FoodLow, AlertKind::FoodLikelyOut]
    );
    assert!(
        alerts
            .open()
            .any(|open| { open.alert == Alert::FoodLikelyOut { confirmed: 5 } })
    );

    alerts.step(6, &calm());
    assert_eq!(kinds(&alerts), []);
}

#[test]
fn the_feed_that_first_reads_low_does_not_count() {
    let mut alerts = settled();

    alerts.event(1, Some(WALL), &done(1, 5, FoodLevel::Low));
    let unknown = Readings {
        food: None,
        ..calm()
    };
    alerts.step(2, &unknown);
    assert_eq!(kinds(&alerts), []);
}

/// The rules look at the beam once a second through a feed, and it may read
/// low part way through: that feed's portions still count for nothing.
#[test]
fn a_low_reading_during_a_feed_counts_none_of_its_portions() {
    let mut alerts = settled();
    let low_in_a_feed = Readings {
        food: Some(false),
        feeding: true,
        ..calm()
    };
    let low = Readings {
        food: Some(false),
        ..calm()
    };

    alerts.step(1, &low_in_a_feed);
    assert_eq!(kinds(&alerts), [AlertKind::FoodLow]);
    alerts.event(2, Some(WALL), &done(1, 5, FoodLevel::Low));
    alerts.step(3, &low);
    assert_eq!(kinds(&alerts), [AlertKind::FoodLow]);

    alerts.event(4, Some(WALL), &done(2, 5, FoodLevel::Low));
    alerts.step(5, &low);
    assert_eq!(
        kinds(&alerts),
        [AlertKind::FoodLow, AlertKind::FoodLikelyOut]
    );
}

#[test]
fn the_food_count_comes_back_from_the_events_the_boot_read_finds() {
    let mut alerts = Alerts::new();

    alerts.event(
        0,
        Some(WALL),
        &EventBody::Food {
            level: FoodLevel::Low,
        },
    );
    alerts.event(0, Some(WALL), &done(1, 3, FoodLevel::Low));
    alerts.event(0, Some(WALL), &done(2, 2, FoodLevel::Low));
    alerts.restored(0, History::Whole);

    // The beam hasn't read since this boot.
    let unknown = Readings {
        food: None,
        ..calm()
    };
    alerts.step(1, &unknown);
    assert_eq!(kinds(&alerts), [AlertKind::FoodLikelyOut]);
    assert!(alerts.settling());
}

#[test]
fn a_refill_between_feeds_starts_the_count_over() {
    let mut alerts = settled();
    let low = Readings {
        food: Some(false),
        ..calm()
    };

    alerts.event(
        1,
        Some(WALL),
        &EventBody::Food {
            level: FoodLevel::Low,
        },
    );
    alerts.event(2, Some(WALL), &done(1, 4, FoodLevel::Low));
    alerts.event(
        3,
        Some(WALL),
        &EventBody::Food {
            level: FoodLevel::Present,
        },
    );
    alerts.step(4, &low);
    alerts.event(5, Some(WALL), &done(2, 4, FoodLevel::Low));
    alerts.step(6, &low);

    assert_eq!(kinds(&alerts), [AlertKind::FoodLow]);
}

#[test]
fn desiccant_due_opens_ends_and_holds_with_no_clock() {
    let mut alerts = settled();
    let with = |desiccant| Readings {
        desiccant,
        ..calm()
    };

    alerts.step(1, &with(Desiccant::Remaining { days: 1 }));
    assert_eq!(kinds(&alerts), []);

    alerts.step(2, &with(Desiccant::Remaining { days: 0 }));
    assert_eq!(kinds(&alerts), [AlertKind::DesiccantDue]);

    alerts.step(3, &with(Desiccant::Unknown));
    assert_eq!(kinds(&alerts), [AlertKind::DesiccantDue], "it holds");

    alerts.step(4, &with(Desiccant::Remaining { days: 14 }));
    assert_eq!(kinds(&alerts), []);

    alerts.step(5, &with(Desiccant::Remaining { days: -1 }));
    alerts.step(6, &with(Desiccant::Unset));
    assert_eq!(kinds(&alerts), []);
}

#[test]
fn battery_below_floor_follows_the_verdict_and_no_verdict_changes_nothing() {
    let mut alerts = settled();
    let verdict = |below_floor| Readings {
        supply: Supply {
            adapter: Some(false),
            battery: Battery::Reading {
                mv: 4000,
                percent: 10,
            },
            below_floor,
        },
        ..calm()
    };

    alerts.step(1, &verdict(Some(true)));
    assert_eq!(kinds(&alerts), [AlertKind::BatteryBelowFloor]);

    alerts.step(2, &verdict(None));
    assert_eq!(kinds(&alerts), [AlertKind::BatteryBelowFloor]);

    alerts.step(3, &verdict(Some(false)));
    assert_eq!(kinds(&alerts), []);
}

// Room.

/// Fills the table with `count` urgent reports.
fn urgent_reports(alerts: &mut Alerts, count: u32) {
    for feed in 0..count {
        alerts.event(1, Some(WALL), &failed(feed));
    }
}

fn feeds(alerts: &Alerts) -> Vec<u32> {
    alerts
        .open()
        .filter_map(|open| match open.alert {
            Alert::MealFailed { feed, .. } => Some(feed),
            _ => None,
        })
        .collect()
}

#[test]
fn the_oldest_of_the_least_urgent_report_leaves_first() {
    let mut alerts = settled();

    alerts.event(1, Some(WALL), &adapter(false));
    let (at, body) = joined(WALL);
    alerts.event(1, at, &body);
    alerts.event(1, Some(WALL), &stopped(UpdateStop::Hash));
    urgent_reports(&mut alerts, 9);
    assert_eq!(alerts.open().count(), CAPACITY);
    assert!(!alerts.more());

    // The power report is the oldest of the two least urgent.
    alerts.event(2, Some(WALL), &failed(100));
    assert_eq!(alerts.open().count(), CAPACITY);
    assert!(!kinds(&alerts).contains(&AlertKind::PowerWasOut));
    assert!(kinds(&alerts).contains(&AlertKind::UpdateFailed));
    assert!(alerts.more());

    // Then the update's, and only then the security report.
    alerts.event(3, Some(WALL), &failed(101));
    assert!(!kinds(&alerts).contains(&AlertKind::UpdateFailed));
    assert!(kinds(&alerts).contains(&AlertKind::ClientJoined));

    alerts.event(4, Some(WALL), &failed(102));
    assert!(!kinds(&alerts).contains(&AlertKind::ClientJoined));
}

#[test]
fn urgent_reports_push_out_the_oldest_urgent_report() {
    let mut alerts = settled();
    urgent_reports(&mut alerts, 12);

    alerts.event(2, Some(WALL), &failed(100));

    let held = feeds(&alerts);
    assert_eq!(held.len(), CAPACITY);
    assert_eq!(held[0], 1, "the oldest went");
    assert_eq!(held[CAPACITY - 1], 100, "the newest is last");
}

#[test]
fn a_report_less_urgent_than_all_the_others_is_dropped_itself() {
    let mut alerts = settled();
    urgent_reports(&mut alerts, 12);
    let before = feeds(&alerts);

    alerts.event(2, Some(WALL), &adapter(false));

    assert_eq!(feeds(&alerts), before);
    assert!(alerts.more());
}

#[test]
fn a_condition_always_finds_room_and_is_never_pushed_out() {
    let mut alerts = settled();
    urgent_reports(&mut alerts, 12);

    // The most that can be open at once.
    for _ in 0..3 {
        alerts.event(1, None, &boot(1, WATCHDOG));
        alerts.event(1, None, &held(UpdateCandidate::None));
    }
    alerts.event(
        1,
        Some(WALL),
        &EventBody::Food {
            level: FoodLevel::Low,
        },
    );
    alerts.event(1, Some(WALL), &done(99, 5, FoodLevel::Low));
    let all = Readings {
        wall_s: None,
        zoned: false,
        blocked: Some(Blocked::Fault),
        food: Some(false),
        feeding: false,
        desiccant: Desiccant::Remaining { days: 0 },
        supply: Supply {
            below_floor: Some(true),
            ..calm().supply
        },
        claim: false,
        plans_settled: true,
    };
    alerts.step(0, &all);
    alerts.step(NO_CLOCK_MS, &all);

    let open = kinds(&alerts);
    assert_eq!(
        open.iter().filter(|kind| kind.is_condition()).count(),
        CONDITIONS
    );
    assert_eq!(open.len(), CAPACITY);

    urgent_reports(&mut alerts, 20);
    let open = kinds(&alerts);
    assert_eq!(
        open.iter().filter(|kind| kind.is_condition()).count(),
        CONDITIONS
    );
    assert_eq!(open.len(), CAPACITY);
}

#[test]
fn more_clears_once_what_left_could_no_longer_have_been_listed() {
    let mut alerts = settled();
    urgent_reports(&mut alerts, 13);
    assert!(alerts.more());

    let far = Readings {
        wall_s: None,
        ..calm()
    };
    alerts.step(MORE_MS, &far);
    assert!(alerts.more());

    let generation = alerts.generation();
    alerts.step(MORE_MS + 1, &far);
    assert!(!alerts.more());
    assert_ne!(alerts.generation(), generation);
}

// The generation and the flags.

#[test]
fn the_generation_moves_with_the_table_and_only_then() {
    let mut alerts = settled();
    let start = alerts.generation();

    alerts.step(1, &calm());
    alerts.event(2, Some(WALL), &done(1, 2, FoodLevel::Present));
    assert_eq!(alerts.generation(), start, "nothing to tell");

    alerts.event(3, Some(WALL), &failed(4));
    let opened = alerts.generation();
    assert_ne!(opened, start);

    alerts.event(4, Some(WALL), &failed(4));
    assert_eq!(alerts.generation(), opened, "the same alert, read back");

    let later = Readings {
        wall_s: Some(WALL + REPORT_LIFE_S),
        ..calm()
    };
    alerts.step(5, &later);
    assert_ne!(alerts.generation(), opened, "it lapsed");
}

#[test]
fn the_table_stays_oldest_first_as_entries_leave() {
    let mut alerts = settled();
    urgent_reports(&mut alerts, 3);
    alerts.event(1, Some(WALL + REPORT_LIFE_S), &failed(9));

    let later = Readings {
        wall_s: Some(WALL + REPORT_LIFE_S),
        ..calm()
    };
    alerts.step(2, &later);
    assert_eq!(feeds(&alerts), [9]);

    alerts.event(3, Some(WALL + REPORT_LIFE_S), &failed(10));
    assert_eq!(feeds(&alerts), [9, 10]);
}

#[test]
fn settling_holds_until_the_boot_read_and_the_boots_thresholds_are_done() {
    let mut alerts = Alerts::new();
    assert!(alerts.settling());

    let cold = Readings {
        wall_s: None,
        blocked: Some(Blocked::Starting),
        ..calm()
    };
    alerts.step(0, &cold);
    assert!(alerts.settling(), "no boot read yet");

    alerts.restored(0, History::Whole);
    alerts.step(1, &cold);
    assert!(alerts.settling(), "the thresholds run");

    let no_clock = Readings {
        wall_s: None,
        ..calm()
    };
    alerts.step(2, &no_clock);
    assert!(alerts.settling(), "the clock's threshold runs still");

    alerts.step(3, &calm());
    assert!(!alerts.settling());
}

#[test]
fn a_threshold_past_at_boot_ends_settling_too() {
    let mut alerts = Alerts::new();
    alerts.restored(0, History::Whole);
    let no_clock = Readings {
        wall_s: None,
        ..calm()
    };

    alerts.step(0, &no_clock);
    assert!(alerts.settling());

    alerts.step(NO_CLOCK_MS, &no_clock);
    assert_eq!(kinds(&alerts), [AlertKind::NoClock]);
    assert!(!alerts.settling());
}

#[test]
fn a_threshold_that_starts_after_settling_does_not_set_it_again() {
    let mut alerts = settled();
    let start = alerts.generation();
    let blocked = Readings {
        blocked: Some(Blocked::Feeding),
        ..calm()
    };

    alerts.step(1, &blocked);

    assert!(!alerts.settling());
    assert_eq!(alerts.generation(), start, "a feed moves nothing");
}

#[test]
fn an_unknown_reading_sets_settling_for_as_long_as_it_lasts() {
    let mut alerts = settled();
    let start = alerts.generation();

    for unknown in [
        Readings {
            desiccant: Desiccant::Unknown,
            ..calm()
        },
        Readings {
            food: None,
            ..calm()
        },
        Readings {
            supply: Supply {
                adapter: None,
                battery: Battery::Unread,
                below_floor: None,
            },
            ..calm()
        },
    ] {
        alerts.step(1, &unknown);
        assert!(alerts.settling(), "{unknown:?}");
        alerts.step(2, &calm());
        assert!(!alerts.settling(), "{unknown:?}");
    }

    assert_ne!(alerts.generation(), start, "the flag is part of a read");
}

#[test]
fn no_pack_fitted_is_no_unknown_reading() {
    let mut alerts = settled();

    alerts.step(1, &calm());

    assert!(!alerts.settling());
}

#[test]
fn a_new_table_is_empty_and_lists_nothing() {
    let alerts = Alerts::default();

    assert_eq!(alerts.generation(), 0);
    assert!(!alerts.more());
    assert_eq!(alerts.open().count(), 0);
}
