use wellfed_model::{
    Alert, AlertsView, Blocked, ClientRole, EnrollPath, FeedOutcome, FeedReason, Generation, Hex,
    MissedReason, OpenAlert,
};

use super::*;

const FEEDER: [u8; 32] = [7; 32];
const OWN: Hex<4> = Hex([0xaa, 0xbb, 0xcc, 0xdd]);

fn at(second: i64) -> Timestamp {
    Timestamp::from_second(1_800_000_000 + second).unwrap()
}

/// A catch-up whose status showed no valid clock.
fn clockless(now: Timestamp, mutes: &Mutes) -> Context<'_> {
    Context {
        clock: FeederClock::Unknown,
        ..context(now, mutes)
    }
}

fn context(now: Timestamp, mutes: &Mutes) -> Context<'_> {
    Context {
        now,
        mutes,
        own: OWN,
        clock: FeederClock::Known,
    }
}

fn cursor(epoch: u32, boot: u32, generation: u32) -> Cursor {
    Cursor {
        epoch,
        boot,
        generation: Generation(generation),
    }
}

fn view(epoch: u32, alerts: &[OpenAlert]) -> AlertsView {
    AlertsView {
        epoch,
        generation: Generation(0),
        more: false,
        settling: false,
        alerts: alerts.to_vec(),
        unread: 0,
    }
}

fn failed(feed: u32) -> OpenAlert {
    OpenAlert {
        at: Some(at(0)),
        alert: Alert::MealFailed {
            feed,
            plan: Some(0),
            requested: 2,
            completed: 0,
            outcome: FeedOutcome::Failed,
            reason: FeedReason::Jammed,
        },
    }
}

fn uncertain(feed: u32) -> OpenAlert {
    OpenAlert {
        at: Some(at(0)),
        alert: Alert::MealUncertain {
            feed,
            plan: Some(0),
            requested: 2,
            completed: 1,
            reason: FeedReason::Interrupted,
        },
    }
}

fn condition(alert: Alert) -> OpenAlert {
    OpenAlert {
        at: Some(at(0)),
        alert,
    }
}

fn posts(watch: &Watch) -> Vec<NoticeKind> {
    watch
        .changes()
        .iter()
        .filter_map(|change| match change {
            Change::Post(tracked) => Some(tracked.notice.kind()),
            Change::Remove(_) => None,
        })
        .collect()
}

fn removals(watch: &Watch) -> usize {
    watch
        .changes()
        .iter()
        .filter(|change| matches!(change, Change::Remove(_)))
        .count()
}

/// Confirm everything the watch hands over, as the app does once it has
/// posted and removed the notices.
fn deliver(watch: &mut Watch) {
    for change in watch.changes() {
        watch.confirm(&change);
    }
}

#[test]
fn an_alert_posts_once_however_often_it_is_read() {
    let mutes = Mutes::default();
    let mut watch = Watch::default();
    let now = context(at(10), &mutes);

    assert!(watch.needs_read(cursor(1, 4, 1)));
    watch.apply(cursor(1, 4, 1), &view(1, &[failed(3)]), &now);
    assert_eq!(posts(&watch), [NoticeKind::MealFailed]);
    deliver(&mut watch);

    assert!(!watch.needs_read(cursor(1, 4, 1)), "the cursor holds");
    watch.apply(cursor(1, 4, 2), &view(1, &[failed(3)]), &now);
    assert!(watch.changes().is_empty(), "read again, not posted again");
    assert_eq!(watch.listed().count(), 1);
}

#[test]
fn an_undelivered_post_is_handed_over_again() {
    let mutes = Mutes::default();
    let mut watch = Watch::default();
    watch.apply(
        cursor(1, 1, 1),
        &view(1, &[failed(3)]),
        &context(at(10), &mutes),
    );
    assert_eq!(posts(&watch), [NoticeKind::MealFailed]);

    // The catch-up stopped before the app confirmed: the next one posts it.
    watch.apply(
        cursor(1, 1, 1),
        &view(1, &[failed(3)]),
        &context(at(20), &mutes),
    );
    assert_eq!(posts(&watch), [NoticeKind::MealFailed]);
}

#[test]
fn a_generation_that_counts_again_after_a_boot_still_reads() {
    let watch = Watch {
        cursor: Some(cursor(1, 4, 2)),
        ..Watch::default()
    };
    assert!(watch.needs_read(cursor(1, 5, 2)), "another boot");
    assert!(watch.needs_read(cursor(2, 4, 2)), "another epoch");
}

#[test]
fn a_condition_comes_off_when_the_feeder_clears_it() {
    let mutes = Mutes::default();
    let mut watch = Watch::default();
    let now = context(at(10), &mutes);
    watch.apply(
        cursor(1, 1, 1),
        &view(1, &[condition(Alert::FoodLow)]),
        &now,
    );
    deliver(&mut watch);

    watch.apply(cursor(1, 1, 2), &view(1, &[]), &now);
    assert_eq!(removals(&watch), 1);
    assert_eq!(watch.listed().count(), 0);
    deliver(&mut watch);
    assert!(watch.changes().is_empty());

    // It opens again: a new notice.
    watch.apply(
        cursor(1, 1, 3),
        &view(1, &[condition(Alert::FoodLow)]),
        &now,
    );
    assert_eq!(posts(&watch), [NoticeKind::FoodLow]);
}

#[test]
fn an_absent_alert_stays_while_the_feeder_cannot_vouch_for_its_end() {
    let mutes = Mutes::default();
    let mut watch = Watch::default();
    let now = context(at(10), &mutes);
    let both = [failed(3), condition(Alert::NoClock)];
    watch.apply(cursor(1, 1, 1), &view(1, &both), &now);
    deliver(&mut watch);

    // Settling after a boot: neither has ended as far as anyone knows.
    let mut settling = view(1, &[]);
    settling.settling = true;
    watch.apply(cursor(1, 2, 0), &settling, &now);
    assert_eq!(watch.listed().count(), 2);
    assert!(watch.changes().is_empty());

    // `more`: a report may only have left for room, but a condition is
    // vouched for.
    let mut more = view(1, &[]);
    more.more = true;
    watch.apply(cursor(1, 2, 1), &more, &now);
    let left: Vec<_> = watch
        .listed()
        .map(|tracked| tracked.notice.kind())
        .collect();
    assert_eq!(left, [NoticeKind::MealFailed]);

    // Past the report's life it has lapsed, whatever the table says.
    watch.lapse(&context(
        at(0) + REPORT_LIFE + SignedDuration::from_secs(1),
        &mutes,
    ));
    assert_eq!(watch.listed().count(), 0);
}

#[test]
fn an_uncertain_feed_found_failed_posts_again_under_its_own_id() {
    let mutes = Mutes::default();
    let mut watch = Watch::default();
    let now = context(at(10), &mutes);
    watch.apply(cursor(1, 1, 1), &view(1, &[uncertain(9)]), &now);
    let first = watch.open[0].key;
    deliver(&mut watch);

    watch.apply(cursor(1, 1, 2), &view(1, &[failed(9)]), &now);
    assert_eq!(posts(&watch), [NoticeKind::MealFailed]);
    assert_eq!(watch.open.len(), 1);
    assert_eq!(watch.open[0].key, first, "one alert, one notification id");
}

#[test]
fn a_posted_alert_whose_new_kind_is_muted_comes_off() {
    let mutes = Mutes {
        kinds: vec![NoticeKind::MealFailed],
        levels: Vec::new(),
    };
    let mut watch = Watch::default();
    let now = context(at(10), &mutes);
    watch.apply(cursor(1, 1, 1), &view(1, &[uncertain(9)]), &now);
    deliver(&mut watch);

    watch.apply(cursor(1, 1, 2), &view(1, &[failed(9)]), &now);
    assert_eq!(removals(&watch), 1, "the uncertain notice is not left up");
    assert!(posts(&watch).is_empty());
    assert_eq!(watch.open[0].delivery, Delivery::Quiet);
}

#[test]
fn a_quiet_alert_that_ends_still_comes_off() {
    let mutes = Mutes {
        kinds: vec![NoticeKind::FoodLow],
        levels: Vec::new(),
    };
    let mut watch = Watch::default();
    let now = context(at(10), &mutes);
    watch.apply(
        cursor(1, 1, 1),
        &view(1, &[condition(Alert::FoodLow)]),
        &now,
    );
    assert!(watch.changes().is_empty());

    watch.apply(cursor(1, 1, 2), &view(1, &[]), &now);
    assert_eq!(removals(&watch), 1);
}

#[test]
fn a_cleared_condition_comes_off_while_the_feeder_settles_for_good() {
    let mutes = Mutes::default();
    let mut watch = Watch::default();
    watch.apply(
        cursor(1, 1, 1),
        &view(1, &[condition(Alert::FoodLow)]),
        &context(at(0), &mutes),
    );
    deliver(&mut watch);

    // No clock: the desiccant's reading stays unknown, so every table
    // settles, and FoodLow has been cleared.
    let mut settling = view(1, &[]);
    settling.settling = true;
    watch.apply(cursor(1, 1, 2), &settling, &clockless(at(60), &mutes));
    assert_eq!(watch.listed().count(), 1, "held while a boot may settle");

    watch.lapse(&clockless(at(60) + SETTLING_HOLD, &mutes));
    assert_eq!(watch.listed().count(), 1, "not before the hold is over");
    watch.lapse(&clockless(at(61) + SETTLING_HOLD, &mutes));
    assert_eq!(watch.listed().count(), 0);
    assert_eq!(removals(&watch), 1);
}

#[test]
fn a_condition_that_waits_for_the_clock_stays_while_there_is_none() {
    let mutes = Mutes::default();
    let mut watch = Watch::default();
    let due = condition(Alert::DesiccantDue { days: -1 });
    watch.apply(cursor(1, 1, 1), &view(1, &[due]), &context(at(0), &mutes));
    deliver(&mut watch);
    let key = watch.open[0].key;
    assert!(watch.dismiss(key));
    deliver(&mut watch);

    // A reboot with no clock: its days are unknown, so the table leaves it
    // out and settles, for as long as the clock is missing.
    let mut settling = view(1, &[]);
    settling.settling = true;
    watch.apply(cursor(1, 2, 0), &settling, &clockless(at(60), &mutes));
    for hours in 1..12 {
        watch.lapse(&clockless(
            at(60) + SignedDuration::from_hours(hours),
            &mutes,
        ));
    }
    assert_eq!(watch.open.len(), 1, "never ended by its hold with no clock");
    assert!(watch.changes().is_empty());

    // The clock comes back and the feeder lists it again: still dismissed,
    // so nothing posts.
    let back = at(60) + SignedDuration::from_hours(12);
    watch.apply(cursor(1, 2, 1), &view(1, &[due]), &context(back, &mutes));
    assert!(watch.changes().is_empty());
    assert_eq!(watch.open[0].delivery, Delivery::Dismissed);
}

#[test]
fn a_condition_that_waits_for_the_clock_counts_its_hold_from_the_clock() {
    let mutes = Mutes::default();
    let mut watch = Watch::default();
    watch.apply(
        cursor(1, 1, 1),
        &view(1, &[condition(Alert::PlansStuck)]),
        &context(at(0), &mutes),
    );
    deliver(&mut watch);

    let mut settling = view(1, &[]);
    settling.settling = true;
    watch.apply(cursor(1, 2, 0), &settling, &clockless(at(60), &mutes));
    let clock = at(60) + SignedDuration::from_hours(5);
    watch.lapse(&context(clock, &mutes));
    assert_eq!(
        watch.open.len(),
        1,
        "its threshold runs from the clock's arrival"
    );

    watch.lapse(&context(clock + SETTLING_HOLD, &mutes));
    assert_eq!(watch.open.len(), 1);
    watch.lapse(&context(
        clock + SETTLING_HOLD + SignedDuration::from_secs(1),
        &mutes,
    ));
    assert_eq!(watch.open.len(), 0);
}

/// A feeder with no desiccant count and no pack, whose clock is valid but
/// has no zone offset: its status shows a clock, and only the table's
/// `NoClock` says it has no local time, so stuck plans can't be judged.
#[test]
fn a_clock_with_no_zone_holds_stuck_plans_as_no_clock_does() {
    let mutes = Mutes::default();
    let mut watch = Watch::default();
    watch.apply(
        cursor(1, 1, 1),
        &view(1, &[condition(Alert::PlansStuck)]),
        &context(at(0), &mutes),
    );
    deliver(&mut watch);

    // A reboot that kept the clock and lost the zone: the feeder settles
    // for as long as it has no local time, and lists NoClock after an hour.
    let mut zoneless = view(1, &[condition(Alert::NoClock)]);
    zoneless.settling = true;
    watch.apply(cursor(1, 2, 1), &zoneless, &context(at(3600), &mutes));
    deliver(&mut watch);
    for hours in 2..24 {
        watch.lapse(&context(at(0) + SignedDuration::from_hours(hours), &mutes));
    }
    let stuck = |watch: &Watch| {
        watch
            .open
            .iter()
            .any(|tracked| tracked.notice.kind() == NoticeKind::PlansStuck)
    };
    assert!(stuck(&watch), "never ended while there is no local time");

    // Local time back: its hold counts from here.
    let mut back = view(1, &[]);
    back.settling = true;
    let zoned = at(0) + SignedDuration::from_hours(24);
    watch.apply(cursor(1, 2, 2), &back, &context(zoned, &mutes));
    watch.lapse(&context(zoned + SETTLING_HOLD, &mutes));
    assert!(stuck(&watch));
    watch.lapse(&context(
        zoned + SETTLING_HOLD + SignedDuration::from_secs(1),
        &mutes,
    ));
    assert!(!stuck(&watch));
}

#[test]
fn a_condition_listed_again_starts_its_hold_over() {
    let mutes = Mutes::default();
    let mut watch = Watch::default();
    let food = view(1, &[condition(Alert::FoodLow)]);
    watch.apply(cursor(1, 1, 1), &food, &context(at(0), &mutes));
    let mut settling = view(1, &[]);
    settling.settling = true;
    watch.apply(cursor(1, 1, 2), &settling, &context(at(60), &mutes));
    watch.apply(cursor(1, 1, 3), &food, &context(at(120), &mutes));

    watch.lapse(&context(at(61) + SETTLING_HOLD, &mutes));
    assert_eq!(watch.listed().count(), 1);
}

#[test]
fn a_report_with_no_clock_lapses_by_how_long_it_has_been_absent() {
    let mutes = Mutes::default();
    let mut watch = Watch::default();
    let mut timeless = failed(4);
    timeless.at = None;
    watch.apply(
        cursor(1, 1, 1),
        &view(1, &[timeless]),
        &context(at(0), &mutes),
    );
    deliver(&mut watch);

    // Left out of a table that has more to say: it may only have left
    // for room.
    let mut more = view(1, &[]);
    more.more = true;
    watch.apply(cursor(1, 1, 2), &more, &context(at(100), &mutes));
    watch.lapse(&context(at(100) + REPORT_LIFE, &mutes));
    assert_eq!(watch.listed().count(), 1);

    watch.lapse(&context(at(101) + REPORT_LIFE, &mutes));
    assert_eq!(watch.listed().count(), 0);
}

#[test]
fn new_words_for_a_condition_update_the_list_and_post_nothing() {
    let mutes = Mutes::default();
    let mut watch = Watch::default();
    let now = context(at(10), &mutes);
    let days = |days| condition(Alert::DesiccantDue { days });
    watch.apply(cursor(1, 1, 1), &view(1, &[days(0)]), &now);
    deliver(&mut watch);

    watch.apply(cursor(1, 1, 2), &view(1, &[days(-1)]), &now);
    assert!(watch.changes().is_empty());
    assert_eq!(
        watch.open[0].notice,
        Notice::Alert { open: days(-1) },
        "the list says what the feeder says now"
    );
}

#[test]
fn another_epoch_ends_what_the_old_ring_said() {
    let mutes = Mutes::default();
    let mut watch = Watch::default();
    let now = context(at(10), &mutes);
    watch.apply(cursor(1, 1, 1), &view(1, &[failed(1)]), &now);
    deliver(&mut watch);

    // A factory reset: feed 1 of the new ring is another feed.
    watch.apply(cursor(2, 2, 1), &view(2, &[failed(1)]), &now);
    let changes = watch.changes();
    assert!(matches!(
        changes[0],
        Change::Remove(NoticeKey::Alert { epoch: 1, .. })
    ));
    assert!(
        matches!(changes[1], Change::Post(tracked) if matches!(tracked.key, NoticeKey::Alert { epoch: 2, .. }))
    );
    assert_ne!(
        NoticeKey::Alert {
            epoch: 1,
            key: failed(1).key()
        }
        .notification_id(&FEEDER),
        NoticeKey::Alert {
            epoch: 2,
            key: failed(1).key()
        }
        .notification_id(&FEEDER),
    );
}

#[test]
fn a_muted_kind_or_level_is_listed_and_never_posted() {
    let mutes = Mutes {
        kinds: vec![NoticeKind::FoodLow],
        levels: vec![AlertLevel::Security],
    };
    let mut watch = Watch::default();
    let joined = condition(Alert::ClientJoined {
        client: Hex([1, 2, 3, 4]),
        role: ClientRole::Member,
        path: EnrollPath::Invite,
    });
    let alerts = [condition(Alert::FoodLow), joined, failed(2)];
    watch.apply(cursor(1, 1, 1), &view(1, &alerts), &context(at(10), &mutes));
    assert_eq!(posts(&watch), [NoticeKind::MealFailed]);
    assert_eq!(watch.listed().count(), 3);
}

#[test]
fn muting_takes_a_posted_notice_off_and_unmuting_posts_nothing() {
    let none = Mutes::default();
    let mut watch = Watch::default();
    watch.apply(
        cursor(1, 1, 1),
        &view(1, &[condition(Alert::FoodLow)]),
        &context(at(10), &none),
    );
    deliver(&mut watch);

    let muted = Mutes {
        kinds: vec![NoticeKind::FoodLow],
        levels: Vec::new(),
    };
    watch.remute(&muted);
    assert_eq!(removals(&watch), 1);
    deliver(&mut watch);

    watch.remute(&none);
    assert!(watch.changes().is_empty());
}

#[test]
fn this_phones_own_joining_is_never_posted() {
    let mutes = Mutes::default();
    let mut watch = Watch::default();
    let own = condition(Alert::ClientJoined {
        client: OWN,
        role: ClientRole::Owner,
        path: EnrollPath::Setup,
    });
    watch.apply(cursor(1, 1, 1), &view(1, &[own]), &context(at(10), &mutes));
    assert!(watch.changes().is_empty());
}

#[test]
fn a_dismissed_alert_is_hidden_here_until_it_ends() {
    let mutes = Mutes::default();
    let mut watch = Watch::default();
    let now = context(at(10), &mutes);
    watch.apply(
        cursor(1, 1, 1),
        &view(1, &[condition(Alert::FoodLow)]),
        &now,
    );
    deliver(&mut watch);

    let key = watch.open[0].key;
    assert!(watch.dismiss(key));
    assert_eq!(removals(&watch), 1, "its notice comes off");
    assert_eq!(watch.listed().count(), 0);
    deliver(&mut watch);

    watch.apply(
        cursor(1, 1, 2),
        &view(1, &[condition(Alert::FoodLow)]),
        &now,
    );
    assert!(watch.changes().is_empty(), "still open: still dismissed");

    watch.apply(cursor(1, 1, 3), &view(1, &[]), &now);
    assert_eq!(
        removals(&watch),
        1,
        "an end takes its id off whatever this watch thought was showing"
    );
    deliver(&mut watch);
    watch.apply(
        cursor(1, 1, 4),
        &view(1, &[condition(Alert::FoodLow)]),
        &now,
    );
    assert_eq!(posts(&watch), [NoticeKind::FoodLow], "a new episode");
}

#[test]
fn a_removal_comes_before_a_post_under_the_same_id() {
    let mutes = Mutes::default();
    let mut watch = Watch::default();
    let now = context(at(10), &mutes);
    let cannot = |reason| condition(Alert::CannotFeed { reason });
    watch.apply(cursor(1, 1, 1), &view(1, &[cannot(Blocked::Fault)]), &now);
    deliver(&mut watch);
    watch.apply(cursor(1, 1, 2), &view(1, &[]), &now);
    watch.apply(cursor(1, 1, 3), &view(1, &[cannot(Blocked::Journal)]), &now);

    let changes = watch.changes();
    assert!(matches!(changes[0], Change::Remove(_)));
    assert!(matches!(changes[1], Change::Post(_)));
}

#[test]
fn a_missed_meal_replaces_its_report_of_no_record() {
    let mutes = Mutes::default();
    let mut watch = Watch::default();
    let now = context(at(10), &mutes);
    let due = at(-3600);
    let no_record = condition(Alert::MealNoRecord { plan: 1, due });
    let missed = condition(Alert::MealMissed {
        plan: 1,
        due,
        reason: MissedReason::Late,
    });
    watch.apply(cursor(1, 1, 1), &view(1, &[no_record]), &now);
    deliver(&mut watch);
    watch.apply(cursor(1, 1, 2), &view(1, &[missed]), &now);
    assert_eq!(posts(&watch), [NoticeKind::MealMissed]);
    assert_eq!(watch.open.len(), 1);
}

#[test]
fn unreachable_opens_only_on_the_home_network_after_three_failures_and_ten_minutes() {
    let mutes = Mutes::default();
    let mut watch = Watch::default();
    watch.reached(Some("home".into()), at(0));

    // Away: nothing counts, however long.
    for minute in 1..60 {
        watch.unanswered(Some("cafe"), &context(at(minute * 60), &mutes));
        watch.unanswered(None, &context(at(minute * 60), &mutes));
    }
    assert_eq!(watch.reach.failures, 0);
    assert!(watch.changes().is_empty());

    // Three at home within a few minutes: not yet.
    for minute in 1..=3 {
        watch.unanswered(Some("home"), &context(at(3600 + minute * 60), &mutes));
    }
    assert!(watch.changes().is_empty());

    // Ten minutes from the first: it opens, once.
    watch.unanswered(Some("home"), &context(at(3600 + 11 * 60), &mutes));
    assert_eq!(posts(&watch), [NoticeKind::Unreachable]);
    deliver(&mut watch);
    watch.unanswered(Some("home"), &context(at(3600 + 12 * 60), &mutes));
    assert!(watch.changes().is_empty());

    // Reached again: it comes off, and the count starts over.
    watch.reached(Some("home".into()), at(7200));
    assert_eq!(removals(&watch), 1);
    assert_eq!(watch.reach.failures, 0);
}

#[test]
fn a_feeder_never_reached_is_never_unreachable() {
    let mutes = Mutes::default();
    let mut watch = Watch::default();
    for minute in 0..120 {
        watch.unanswered(Some("home"), &context(at(minute * 60), &mutes));
    }
    assert!(watch.changes().is_empty(), "no network is its home yet");
}

#[test]
fn a_notification_id_is_stable_positive_and_per_feeder() {
    let key = NoticeKey::Alert {
        epoch: 5,
        key: failed(3).key(),
    };
    let id = key.notification_id(&FEEDER);
    assert!(id >= 0x4000_0000);
    assert_eq!(id, key.notification_id(&FEEDER));
    assert_ne!(id, key.notification_id(&[8; 32]));
    assert_ne!(id, NoticeKey::Unreachable.notification_id(&FEEDER));
}

/// Pinned: a notification posted by one build is taken off by the next, so
/// these ids never change.
#[test]
fn notification_ids_are_pinned() {
    let feed = NoticeKey::Alert {
        epoch: 5,
        key: failed(3).key(),
    };
    let food = NoticeKey::Alert {
        epoch: 1,
        key: condition(Alert::FoodLow).key(),
    };
    let alert = |alert: Alert| NoticeKey::Alert {
        epoch: 1,
        key: condition(alert).key(),
    };
    let meal = alert(Alert::MealNoRecord {
        plan: 2,
        due: at(-60),
    });
    let joined = alert(Alert::ClientJoined {
        client: Hex([1, 2, 3, 4]),
        role: ClientRole::Member,
        path: EnrollPath::Invite,
    });
    let power = alert(Alert::PowerWasOut);
    let ids = [
        feed.notification_id(&FEEDER),
        food.notification_id(&FEEDER),
        NoticeKey::Unreachable.notification_id(&FEEDER),
        meal.notification_id(&FEEDER),
        joined.notification_id(&FEEDER),
        power.notification_id(&FEEDER),
    ];
    assert_eq!(
        ids,
        [
            1_116_229_146,
            1_639_503_056,
            1_831_101_205,
            1_862_037_239,
            1_081_698_757,
            1_382_233_608,
        ]
    );
}

#[test]
fn every_kind_has_a_level_and_unreachable_is_urgent() {
    assert_eq!(NoticeKind::all().count(), AlertKind::ALL.len() + 1);
    assert_eq!(NoticeKind::Unreachable.level(), AlertLevel::Urgent);
    assert_eq!(NoticeKind::ClientJoined.level(), AlertLevel::Security);
    assert_eq!(NoticeKind::FoodLow.level(), AlertLevel::Soon);
}

#[test]
fn a_watch_reads_back_as_written() {
    let mutes = Mutes::default();
    let mut watch = Watch::default();
    watch.reached(Some("home".into()), at(0));
    watch.apply(
        cursor(1, 1, 1),
        &view(1, &[failed(3), condition(Alert::NoClock)]),
        &context(at(10), &mutes),
    );
    let json = serde_json::to_string(&watch).unwrap();
    assert_eq!(serde_json::from_str::<Watch>(&json).unwrap(), watch);
}
