use wellfed_model::{
    Battery, Blocked, Desiccant, FeedReason, FoodLevel, Hex, PLANS, Plan, Readiness, Supply,
    Weekdays, WifiSource,
};

use super::*;

const EPOCH: u32 = 7;
const GRACE: SignedDuration = SignedDuration::from_mins(25);

fn t(text: &str) -> Timestamp {
    text.parse().unwrap()
}

const BREAKFAST: Plan = Plan {
    minute: 8 * 60,
    portions: 2,
    days: Weekdays::DAILY,
    enabled: true,
    call: 0,
};

fn plans(generation: u32, table: &[(usize, Plan)]) -> Plans {
    let mut plans = [None; PLANS];
    for (slot, plan) in table {
        plans[*slot] = Some(*plan);
    }
    Plans {
        generation: Generation(generation),
        writable: true,
        plans,
    }
}

fn status(last_event: Option<u64>) -> Status {
    Status {
        build: "test".to_owned(),
        bank: 1,
        uptime_ms: 1_000,
        boot_count: 1,
        reset_flags: 0,
        faults: 0,
        rssi: -50,
        wifi: WifiSource::Stored,
        link_losses: 0,
        dns_losses: 0,
        // The meal rules never read it: they work with no feeder clock.
        clock: None,
        power: None,
        locked: None,
        settings_generation: Generation(1),
        wifi_generation: Generation(1),
        last_event,
        sessions: 1,
        food: FoodLevel::Present,
        feed_fault: None,
        readiness: Some(Readiness::Blocked(Blocked::Feeding)),
        desiccant: Desiccant::Unset,
        supply: Some(Supply {
            adapter: Some(true),
            battery: Battery::Absent,
            below_floor: None,
        }),
    }
}

fn fed_body(feed: u32, outcome: FeedOutcome, reason: FeedReason, by: Source) -> EventBody {
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

fn done_by_plan(feed: u32, plan: u8) -> EventBody {
    fed_body(
        feed,
        FeedOutcome::Done,
        FeedReason::Done,
        Source::Schedule { plan },
    )
}

const CLIENT: Source = Source::Client {
    client: Hex([1, 2, 3, 4]),
};

/// A watcher connected to a feeder, numbering events as the feeder would.
struct Watch {
    engine: Engine,
    zone: TimeZone,
    seq: u64,
}

impl Watch {
    fn new() -> Self {
        Self {
            engine: Engine::default(),
            zone: TimeZone::UTC,
            seq: 0,
        }
    }

    /// Connected and caught up at `now`, with `plans` read.
    fn with_plans(plans: Plans, now: Timestamp) -> Self {
        let mut watch = Self::new();
        watch.see(Observation::Link(Link::Connected), now);
        watch.see(Observation::Plans(plans), now);
        watch.read_status(now);
        watch
    }

    fn see(&mut self, observation: Observation<'_>, now: Timestamp) -> Vec<Change> {
        self.engine.observe(observation, now, &self.zone)
    }

    fn event(&mut self, at: Option<Timestamp>, body: EventBody, now: Timestamp) -> Vec<Change> {
        self.seq += 1;
        let event = Event {
            epoch: EPOCH,
            seq: self.seq,
            at,
            body,
        };
        self.see(Observation::Event(event), now)
    }

    /// A status with every event already delivered.
    fn read_status(&mut self, now: Timestamp) -> Vec<Change> {
        let newest = (self.seq > 0).then_some(self.seq);
        self.see(Observation::Status(&status(newest)), now)
    }

    fn open_kinds(&self) -> Vec<AlertKind> {
        self.engine.open().map(|alert| alert.kind).collect()
    }
}

fn opened(changes: &[Change]) -> Vec<AlertKind> {
    changes
        .iter()
        .filter_map(|change| match change {
            Change::Opened(alert) => Some(alert.kind),
            Change::Cleared(_) => None,
        })
        .collect()
}

fn cleared(changes: &[Change]) -> Vec<AlertKind> {
    changes
        .iter()
        .filter_map(|change| match change {
            Change::Cleared(key) => Some(key.kind),
            Change::Opened(_) => None,
        })
        .collect()
}

fn no_record_key(plan: u8, due: Timestamp) -> Key {
    Key {
        kind: AlertKind::MealNoRecord,
        occurrence: Occurrence::Meal { plan, due },
    }
}

/// The day before the morning every test below runs on.
const START: &str = "2026-10-29T12:00:00Z";
const DUE: &str = "2026-10-30T08:00:00Z";

fn breakfast() -> Watch {
    Watch::with_plans(plans(1, &[(0, BREAKFAST)]), t(START))
}

/// A watcher whose 08:00 meal has reached its grace with no record.
fn no_record() -> Watch {
    let mut watch = breakfast();
    let changes = watch.read_status(t(DUE) + GRACE);
    assert_eq!(opened(&changes), [AlertKind::MealNoRecord]);
    watch
}

#[test]
fn every_kind_has_its_level() {
    assert_eq!(AlertKind::MealFailed.level(), Level::Urgent);
    assert_eq!(AlertKind::MealNoRecord.level(), Level::Urgent);
    assert_eq!(AlertKind::Unreachable.level(), Level::Urgent);
    assert_eq!(AlertKind::FoodLow.level(), Level::Soon);
    assert_eq!(AlertKind::HistoryLost.level(), Level::Soon);
    assert_eq!(AlertKind::ClientJoined.level(), Level::Security);
    assert_eq!(AlertKind::DailyDigest.level(), Level::Optional);
    assert_eq!(AlertKind::ManualFeed.level(), Level::Optional);
}

#[test]
fn a_feed_that_failed_or_expired_opens_with_its_reason_and_portions() {
    let now = t("2026-10-30T08:01:00Z");
    for (feed, outcome, reason) in [
        (1, FeedOutcome::Failed, FeedReason::Jammed),
        (2, FeedOutcome::Expired, FeedReason::Timeout),
    ] {
        let mut watch = breakfast();
        let body = fed_body(feed, outcome, reason, Source::Schedule { plan: 0 });
        let changes = watch.event(Some(now), body, now);

        let [Change::Opened(alert)] = changes[..] else {
            panic!("{changes:?}");
        };
        assert_eq!(alert.kind, AlertKind::MealFailed);
        assert_eq!(alert.level, Level::Urgent);
        assert_eq!(alert.opened_at, now);
        let Detail::Feed(detail) = alert.detail else {
            panic!("{alert:?}");
        };
        assert_eq!(detail.reason, reason);
        assert_eq!(detail.delivered.bounds(), (1, 1));
        assert_eq!(detail.requested, 2);
    }
}

#[test]
fn an_uncertain_feed_opens_once_though_it_records_a_second_fed() {
    let mut watch = breakfast();
    let now = t(DUE);
    let by = Source::Schedule { plan: 0 };
    let first = fed_body(4, FeedOutcome::Uncertain, FeedReason::Interrupted, by);
    let second = fed_body(4, FeedOutcome::Failed, FeedReason::Output, by);

    assert_eq!(
        opened(&watch.event(Some(now), first, now)),
        [AlertKind::MealUncertain]
    );
    assert_eq!(watch.event(Some(now), second, now), []);
    assert_eq!(watch.open_kinds(), [AlertKind::MealUncertain]);
}

#[test]
fn a_scheduled_feed_that_finished_is_optional_and_clears_the_last_meals_alerts() {
    let mut watch = breakfast();
    watch.engine.preferences_mut().enable(AlertKind::MealFed);
    let at = t("2026-10-30T07:00:00Z");
    watch.event(
        Some(at),
        fed_body(1, FeedOutcome::Failed, FeedReason::Jammed, CLIENT),
        at,
    );

    let now = t(DUE);
    let changes = watch.event(Some(now), done_by_plan(2, 0), now);

    assert_eq!(cleared(&changes), [AlertKind::MealFailed]);
    assert_eq!(opened(&changes), [AlertKind::MealFed]);
    assert_eq!(watch.open_kinds(), [AlertKind::MealFed]);
}

#[test]
fn a_manual_feed_is_optional_and_lapses_after_a_day() {
    for by in [CLIENT, Source::Panel] {
        let mut watch = breakfast();
        watch.engine.preferences_mut().enable(AlertKind::ManualFeed);
        let now = t(DUE);
        let body = fed_body(1, FeedOutcome::Done, FeedReason::Done, by);
        assert_eq!(
            opened(&watch.event(Some(now), body, now)),
            [AlertKind::ManualFeed]
        );

        let before = now + SignedDuration::from_hours(24) - SignedDuration::from_secs(1);
        assert_eq!(watch.see(Observation::Tick, before), []);
        let changes = watch.see(Observation::Tick, now + SignedDuration::from_hours(24));
        assert_eq!(cleared(&changes), [AlertKind::ManualFeed]);
    }
}

#[test]
fn a_meal_alert_holds_past_a_day_until_a_meal_goes_right() {
    let mut watch = breakfast();
    let now = t("2026-10-30T07:00:00Z");
    let failed = fed_body(1, FeedOutcome::Failed, FeedReason::Jammed, CLIENT);
    watch.event(Some(now), failed, now);

    let later = now + SignedDuration::from_hours(72);
    assert!(cleared(&watch.see(Observation::Tick, later)).is_empty());
    assert_eq!(watch.open_kinds(), [AlertKind::MealFailed]);
}

#[test]
fn a_cancelled_feed_opens_nothing_and_counts_as_a_record() {
    let mut watch = breakfast();
    let now = t("2026-10-30T08:00:30Z");
    let cancelled = fed_body(
        1,
        FeedOutcome::Cancelled,
        FeedReason::Cancelled,
        Source::Schedule { plan: 0 },
    );

    assert_eq!(watch.event(Some(now), cancelled, now), []);
    assert_eq!(watch.read_status(t(DUE) + GRACE), []);
}

#[test]
fn a_missed_meal_opens_once_with_its_reason() {
    let mut watch = breakfast();
    let now = t("2026-10-30T08:16:00Z");
    let body = EventBody::Missed {
        plan: 0,
        due: t(DUE),
        reason: MissedReason::Late,
    };

    let changes = watch.event(Some(now), body, now);
    let [Change::Opened(alert)] = changes[..] else {
        panic!("{changes:?}");
    };
    assert_eq!(alert.kind, AlertKind::MealMissed);
    assert_eq!(
        alert.detail,
        Detail::Missed {
            plan: 0,
            due: t(DUE),
            reason: MissedReason::Late
        }
    );

    // A restarted feeder says it again under a new number.
    assert_eq!(watch.event(Some(now), body, now), []);
}

#[test]
fn a_meal_with_no_record_opens_exactly_at_due_plus_grace() {
    let mut watch = breakfast();
    let due = t(DUE);

    let early = due + GRACE - SignedDuration::from_secs(1);
    assert_eq!(watch.read_status(early), []);

    let changes = watch.read_status(due + GRACE);
    let [Change::Opened(alert)] = changes[..] else {
        panic!("{changes:?}");
    };
    assert_eq!(alert.key, no_record_key(0, due));
    assert_eq!(
        alert.detail,
        Detail::Meal {
            plan: 0,
            due,
            portions: 2
        }
    );

    assert_eq!(watch.read_status(due + GRACE + GRACE), []);
}

#[test]
fn a_missed_event_clears_and_prevents_no_record() {
    let mut watch = no_record();
    let now = t("2026-10-30T08:30:00Z");
    let body = EventBody::Missed {
        plan: 0,
        due: t(DUE),
        reason: MissedReason::Late,
    };
    let changes = watch.event(Some(now), body, now);
    assert_eq!(cleared(&changes), [AlertKind::MealNoRecord]);
    assert_eq!(opened(&changes), [AlertKind::MealMissed]);

    let mut watch = breakfast();
    let now = t("2026-10-30T08:16:00Z");
    watch.event(Some(now), body, now);
    assert_eq!(watch.read_status(t(DUE) + GRACE), []);
}

#[test]
fn a_late_fed_clears_no_record_whatever_its_outcome() {
    let mut watch = no_record();
    let now = t("2026-10-30T08:40:00Z");
    let body = fed_body(
        1,
        FeedOutcome::Failed,
        FeedReason::Jammed,
        Source::Schedule { plan: 0 },
    );

    let changes = watch.event(Some(now), body, now);
    assert_eq!(cleared(&changes), [AlertKind::MealNoRecord]);
    assert_eq!(opened(&changes), [AlertKind::MealFailed]);
}

#[test]
fn a_fed_with_no_time_is_a_record_stamped_when_it_arrived() {
    let mut watch = breakfast();
    let now = t("2026-10-30T08:10:00Z");

    let changes = watch.event(None, done_by_plan(1, 0), now);
    assert_eq!(changes, []);
    assert_eq!(watch.read_status(t(DUE) + GRACE), []);

    let mut watch = no_record();
    let now = t("2026-10-30T09:00:00Z");
    let changes = watch.event(None, done_by_plan(1, 0), now);
    assert_eq!(cleared(&changes), [AlertKind::MealNoRecord]);
}

#[test]
fn another_plans_feed_does_not_answer_for_this_one() {
    let lunch = Plan {
        minute: 7 * 60,
        ..BREAKFAST
    };
    let mut watch = Watch::with_plans(plans(1, &[(0, BREAKFAST), (1, lunch)]), t(START));
    let now = t("2026-10-30T07:00:30Z");
    watch.event(Some(now), done_by_plan(1, 1), now);

    assert_eq!(
        opened(&watch.read_status(t(DUE) + GRACE)),
        [AlertKind::MealNoRecord]
    );
    assert_eq!(watch.open_kinds(), [AlertKind::MealNoRecord]);
}

#[test]
fn the_next_good_meal_closes_a_no_record_nothing_else_will() {
    let mut watch = no_record();
    let now = t("2026-10-31T08:00:30Z");

    let changes = watch.event(Some(now), done_by_plan(1, 0), now);
    assert_eq!(cleared(&changes), [AlertKind::MealNoRecord]);
    assert!(watch.open_kinds().is_empty());
}

#[test]
fn a_plan_added_after_its_time_is_not_a_missed_meal() {
    let added = t("2026-10-30T14:00:00Z");
    let mut watch = breakfast();
    let cancelled = Plan {
        enabled: false,
        ..BREAKFAST
    };
    watch.see(Observation::Plans(plans(1, &[(0, cancelled)])), t(DUE));
    assert_eq!(watch.read_status(t(DUE) + GRACE), []);

    watch.see(Observation::Plans(plans(2, &[(0, BREAKFAST)])), added);
    assert_eq!(watch.read_status(added + GRACE), []);

    // The next one counts.
    assert_eq!(
        opened(&watch.read_status(t("2026-10-31T08:25:00Z"))),
        [AlertKind::MealNoRecord]
    );
}

#[test]
fn the_first_plans_seen_count_only_from_then_on() {
    let now = t("2026-10-30T14:00:00Z");
    let mut watch = Watch::with_plans(plans(1, &[(0, BREAKFAST)]), now);

    assert_eq!(watch.read_status(now), []);
}

#[test]
fn a_once_plan_that_will_run_tomorrow_is_not_todays_meal() {
    let once = Plan {
        days: Weekdays::ONCE,
        ..BREAKFAST
    };
    let saved = t("2026-10-30T09:00:00Z");
    let mut watch = Watch::with_plans(plans(1, &[(0, once)]), saved);
    assert_eq!(watch.read_status(saved + GRACE), []);

    // It ran, and the feeder disabled it.
    let ran = t("2026-10-31T08:00:30Z");
    watch.event(Some(ran), done_by_plan(1, 0), ran);
    let spent = Plan {
        enabled: false,
        ..once
    };
    watch.see(Observation::Plans(plans(2, &[(0, spent)])), ran);

    assert_eq!(watch.read_status(t("2026-11-01T08:25:00Z")), []);
}

#[test]
fn an_event_that_says_the_plans_changed_holds_until_they_are_read() {
    let mut watch = breakfast();
    let now = t("2026-10-30T07:00:00Z");
    let body = EventBody::Plans {
        generation: Generation(2),
        by: CLIENT,
    };
    watch.event(Some(now), body, now);
    assert_eq!(watch.read_status(t(DUE) + GRACE), []);

    // Nor does a replay of an older generation make the table stale.
    let mut watch = breakfast();
    let body = EventBody::Plans {
        generation: Generation(1),
        by: CLIENT,
    };
    watch.event(Some(now), body, now);
    assert_eq!(
        opened(&watch.read_status(t(DUE) + GRACE)),
        [AlertKind::MealNoRecord]
    );
}

#[test]
fn no_record_holds_while_the_link_is_down_and_opens_when_it_is_known() {
    for link in [Link::Failed, Link::Away] {
        let mut watch = breakfast();
        watch.see(Observation::Link(link), t("2026-10-30T07:59:00Z"));
        assert_eq!(watch.see(Observation::Tick, t(DUE) + GRACE), []);

        let back = t("2026-10-30T09:00:00Z");
        assert_eq!(watch.see(Observation::Link(Link::Connected), back), []);
        assert_eq!(watch.see(Observation::Tick, back), []);
        assert_eq!(opened(&watch.read_status(back)), [AlertKind::MealNoRecord]);
    }
}

#[test]
fn no_record_holds_on_a_status_gone_stale() {
    let mut watch = breakfast();
    watch.read_status(t("2026-10-30T08:09:00Z"));

    assert_eq!(watch.see(Observation::Tick, t(DUE) + GRACE), []);
    assert_eq!(
        opened(&watch.see(Observation::Tick, t("2026-10-30T08:24:30Z"))),
        []
    );
    assert_eq!(
        opened(&watch.read_status(t(DUE) + GRACE)),
        [AlertKind::MealNoRecord]
    );
}

#[test]
fn no_record_holds_until_the_backlog_has_arrived() {
    let mut watch = breakfast();
    let back = t("2026-10-30T09:00:00Z");

    // The status counts a feed this watcher has not been sent yet.
    let behind = status(Some(1));
    assert_eq!(watch.see(Observation::Status(&behind), back), []);

    let fed_at = t("2026-10-30T08:00:30Z");
    assert_eq!(watch.event(Some(fed_at), done_by_plan(1, 0), back), []);
    assert_eq!(watch.read_status(back), []);
    assert!(watch.open_kinds().is_empty());
}

#[test]
fn a_status_while_the_link_stays_up_needs_no_second_connect() {
    let mut watch = breakfast();
    let now = t("2026-10-30T08:24:00Z");
    watch.read_status(now);
    watch.see(Observation::Link(Link::Connected), now);

    assert_eq!(
        opened(&watch.see(Observation::Tick, t(DUE) + GRACE)),
        [AlertKind::MealNoRecord]
    );
}

#[test]
fn no_clock_on_the_feeder_changes_nothing_about_no_record() {
    let mut watch = breakfast();
    assert_eq!(status(None).clock, None);

    assert_eq!(
        opened(&watch.read_status(t(DUE) + GRACE)),
        [AlertKind::MealNoRecord]
    );
}

#[test]
fn no_record_crosses_midnight() {
    let late = Plan {
        minute: 23 * 60 + 50,
        ..BREAKFAST
    };
    let mut watch = Watch::with_plans(plans(1, &[(0, late)]), t("2026-10-30T12:00:00Z"));

    assert_eq!(watch.read_status(t("2026-10-31T00:14:59Z")), []);
    let changes = watch.read_status(t("2026-10-31T00:15:00Z"));
    let [Change::Opened(alert)] = changes[..] else {
        panic!("{changes:?}");
    };
    assert_eq!(alert.key, no_record_key(0, t("2026-10-30T23:50:00Z")));
}

#[test]
fn a_feed_that_ends_past_midnight_is_its_meals_record() {
    let late = Plan {
        minute: 23 * 60 + 59,
        ..BREAKFAST
    };
    let mut watch = Watch::with_plans(plans(1, &[(0, late)]), t(START));
    let ended = t("2026-10-31T00:00:30Z");
    watch.event(Some(ended), done_by_plan(1, 0), ended);

    assert_eq!(watch.read_status(t("2026-10-31T00:30:00Z")), []);
}

#[test]
fn a_hold_past_midnight_still_reaches_yesterdays_meal() {
    let late = Plan {
        minute: 23 * 60 + 50,
        ..BREAKFAST
    };
    let mut watch = Watch::with_plans(plans(1, &[(0, late)]), t(START));
    watch.see(Observation::Link(Link::Failed), t("2026-10-30T23:00:00Z"));
    watch.see(Observation::Tick, t("2026-10-31T00:15:00Z"));

    let changes = watch.read_status(t("2026-10-31T02:00:00Z"));
    let [Change::Opened(alert)] = changes[..] else {
        panic!("{changes:?}");
    };
    assert_eq!(alert.key, no_record_key(0, t("2026-10-30T23:50:00Z")));
}

#[test]
fn on_the_day_clocks_go_forward_a_skipped_meal_is_judged_when_the_feeder_ran_it() {
    let skipped = Plan {
        minute: 2 * 60 + 30,
        ..BREAKFAST
    };
    let mut watch = Watch::with_plans(plans(1, &[(0, skipped)]), t("2026-03-07T12:00:00Z"));
    watch.zone = TimeZone::posix("MST7MDT,M3.2.0,M11.1.0").unwrap();

    // 02:30 does not exist on March 8: the feeder runs it at 03:30 MDT.
    let due = t("2026-03-08T09:30:00Z");
    assert_eq!(watch.read_status(t("2026-03-08T09:54:59Z")), []);

    let changes = watch.read_status(due + GRACE);
    let [Change::Opened(alert)] = changes[..] else {
        panic!("{changes:?}");
    };
    assert_eq!(alert.key, no_record_key(0, due));
}

#[test]
fn on_the_day_clocks_go_back_a_repeated_meal_is_its_first_pass() {
    let repeated = Plan {
        minute: 60 + 30,
        ..BREAKFAST
    };
    let mut watch = Watch::with_plans(plans(1, &[(0, repeated)]), t("2026-10-31T12:00:00Z"));
    watch.zone = TimeZone::posix("MST7MDT,M3.2.0,M11.1.0").unwrap();

    // 01:30 MDT on November 1, not the 01:30 MST an hour later.
    let due = t("2026-11-01T07:30:00Z");
    let changes = watch.read_status(due + GRACE);
    let [Change::Opened(alert)] = changes[..] else {
        panic!("{changes:?}");
    };
    assert_eq!(alert.key, no_record_key(0, due));

    assert_eq!(watch.read_status(due + GRACE + GRACE + GRACE), []);
}

#[test]
fn meal_soon_opens_once_and_clears_when_it_is_due() {
    let mut watch = breakfast();
    watch.engine.preferences_mut().enable(AlertKind::MealSoon);
    let now = t("2026-10-30T07:45:00Z");
    let body = EventBody::MealSoon {
        plan: 0,
        due: t(DUE),
        minute: 480,
        portions: 2,
    };

    assert_eq!(
        opened(&watch.event(Some(now), body, now)),
        [AlertKind::MealSoon]
    );
    // A feeder restart inside the lead sends it again.
    assert_eq!(watch.event(Some(now), body, now), []);

    let changes = watch.read_status(t(DUE));
    assert_eq!(cleared(&changes), [AlertKind::MealSoon]);
}

#[test]
fn meal_soon_read_after_its_time_opens_nothing() {
    let mut watch = breakfast();
    watch.engine.preferences_mut().enable(AlertKind::MealSoon);
    let body = EventBody::MealSoon {
        plan: 0,
        due: t(DUE),
        minute: 480,
        portions: 2,
    };

    assert_eq!(watch.event(None, body, t("2026-10-30T08:05:00Z")), []);
    assert!(watch.open_kinds().is_empty());
}

#[test]
fn a_duplicate_event_is_ignored_and_the_cursor_holds() {
    let mut watch = breakfast();
    let now = t(DUE);
    let event = Event {
        epoch: EPOCH,
        seq: 5,
        at: Some(now),
        body: fed_body(1, FeedOutcome::Failed, FeedReason::Jammed, CLIENT),
    };

    assert_eq!(
        opened(&watch.see(Observation::Event(event), now)),
        [AlertKind::MealFailed]
    );
    let another_feed = Event {
        seq: 4,
        body: fed_body(2, FeedOutcome::Failed, FeedReason::Jammed, CLIENT),
        ..event
    };
    assert_eq!(watch.see(Observation::Event(another_feed), now), []);
    assert_eq!(watch.see(Observation::Event(event), now), []);
    assert_eq!(watch.engine.cursor(), Some((EPOCH, 5)));
}

#[test]
fn a_new_epoch_starts_the_cursor_again_and_keeps_what_is_open() {
    let mut watch = breakfast();
    let now = t("2026-10-30T07:00:00Z");
    let failed = fed_body(1, FeedOutcome::Failed, FeedReason::Jammed, CLIENT);
    let old = Event {
        epoch: EPOCH,
        seq: 90,
        at: Some(now),
        body: failed,
    };
    watch.see(Observation::Event(old), now);

    let new = Event {
        epoch: EPOCH + 1,
        seq: 1,
        ..old
    };
    let changes = watch.see(Observation::Event(new), now);

    // The same feed number is another feed in another history.
    assert_eq!(opened(&changes), [AlertKind::MealFailed]);
    assert_eq!(watch.engine.cursor(), Some((EPOCH + 1, 1)));
    assert_eq!(
        watch.open_kinds(),
        [AlertKind::MealFailed, AlertKind::MealFailed]
    );
}

#[test]
fn a_new_epoch_rules_out_meals_its_ring_no_longer_holds() {
    let mut watch = breakfast();
    let now = t("2026-10-30T07:00:00Z");
    let seen = Event {
        epoch: EPOCH,
        seq: 1,
        at: Some(now),
        body: EventBody::Lost { count: 0 },
    };
    watch.see(Observation::Event(seen), now);
    let wiped = Event {
        epoch: EPOCH + 1,
        ..seen
    };
    let after = t("2026-10-30T08:10:00Z");
    watch.see(Observation::Event(wiped), after);

    let status = status(Some(1));
    assert_eq!(watch.see(Observation::Status(&status), t(DUE) + GRACE), []);
}

#[test]
fn a_gap_moves_the_cursor_and_rules_out_what_fell_in_it() {
    let mut watch = breakfast();
    let now = t("2026-10-30T09:00:00Z");
    let gap = Gap {
        epoch: EPOCH,
        from: 1,
        to: 40,
    };

    watch.see(Observation::Gap(gap), now);
    assert_eq!(watch.engine.cursor(), Some((EPOCH, 40)));
    let status = status(Some(40));
    assert_eq!(watch.see(Observation::Status(&status), now), []);

    let event = Event {
        epoch: EPOCH,
        seq: 40,
        at: Some(now),
        body: fed_body(1, FeedOutcome::Failed, FeedReason::Jammed, CLIENT),
    };
    assert_eq!(watch.see(Observation::Event(event), now), []);
}

#[test]
fn lost_events_rule_out_what_may_have_been_among_them() {
    let mut watch = breakfast();
    let now = t("2026-10-30T09:00:00Z");

    watch.event(Some(now), EventBody::Lost { count: 3 }, now);
    assert_eq!(watch.read_status(now), []);
}

#[test]
fn acknowledging_what_is_over_clears_it_for_good() {
    let mut watch = breakfast();
    let now = t("2026-10-30T08:16:00Z");
    let body = EventBody::Missed {
        plan: 0,
        due: t(DUE),
        reason: MissedReason::Late,
    };
    let changes = watch.event(Some(now), body, now);
    let [Change::Opened(alert)] = changes[..] else {
        panic!("{changes:?}");
    };

    assert_eq!(
        watch.engine.acknowledge(alert.key),
        Some(Change::Cleared(alert.key))
    );
    assert_eq!(watch.engine.open().count(), 0);
    assert_eq!(watch.engine.acknowledge(alert.key), None);
    assert_eq!(watch.event(Some(now), body, now), []);
}

#[test]
fn acknowledging_a_no_record_keeps_it_open_and_it_never_reopens() {
    let mut watch = no_record();
    let key = no_record_key(0, t(DUE));

    assert_eq!(watch.engine.acknowledge(key), None);
    let alert = watch.engine.open().next().unwrap();
    assert!(alert.acknowledged);

    assert_eq!(watch.read_status(t("2026-10-30T08:40:00Z")), []);
    let now = t("2026-10-30T08:50:00Z");
    let changes = watch.event(Some(now), done_by_plan(1, 0), now);
    assert_eq!(cleared(&changes), [AlertKind::MealNoRecord]);
    assert_eq!(watch.read_status(now), []);
}

#[test]
fn a_disabled_kind_is_tracked_but_says_nothing() {
    let mut watch = breakfast();
    assert!(!watch.engine.preferences().enabled(AlertKind::MealFed));
    assert!(watch.engine.preferences().enabled(AlertKind::ClientJoined));
    assert!(watch.engine.preferences().enabled(AlertKind::DailyDigest));

    watch
        .engine
        .preferences_mut()
        .disable(AlertKind::MealFailed);
    let now = t("2026-10-30T07:00:00Z");
    let body = fed_body(1, FeedOutcome::Failed, FeedReason::Jammed, CLIENT);
    assert_eq!(watch.event(Some(now), body, now), []);
    assert_eq!(watch.engine.open().count(), 0);
    assert_eq!(watch.engine.state().open.len(), 1);

    let now = t(DUE);
    assert_eq!(watch.event(Some(now), done_by_plan(2, 0), now), []);
    let tracked: Vec<_> = watch
        .engine
        .state()
        .open
        .iter()
        .map(|open| open.alert.kind)
        .collect();
    assert_eq!(tracked, [AlertKind::MealFed]);
}

#[test]
fn a_kind_turned_off_while_open_still_clears_what_it_announced() {
    let mut watch = no_record();
    watch
        .engine
        .preferences_mut()
        .disable(AlertKind::MealNoRecord);
    let now = t("2026-10-30T08:50:00Z");

    let changes = watch.event(Some(now), done_by_plan(1, 0), now);
    assert_eq!(cleared(&changes), [AlertKind::MealNoRecord]);
}

#[test]
fn a_watcher_resumes_from_its_stored_state_mid_timer() {
    let mut watch = breakfast();
    watch.engine.preferences_mut().enable(AlertKind::ManualFeed);
    watch
        .engine
        .preferences_mut()
        .disable(AlertKind::MealFailed);
    let now = t("2026-10-30T07:00:00Z");
    let manual = fed_body(1, FeedOutcome::Done, FeedReason::Done, CLIENT);
    watch.event(Some(now), manual, now);
    watch.read_status(t("2026-10-30T08:20:00Z"));

    let json = serde_json::to_string(watch.engine.state()).unwrap();
    let state: State = serde_json::from_str(&json).unwrap();
    assert_eq!(&state, watch.engine.state());

    let mut resumed = Watch {
        engine: Engine::resume(state, Thresholds::default()),
        zone: TimeZone::UTC,
        seq: watch.seq,
    };
    assert_eq!(resumed.engine.cursor(), watch.engine.cursor());
    assert!(!resumed.engine.preferences().enabled(AlertKind::MealFailed));

    // The grace and the lapse both go on from where they were, and what
    // opened before the restart does not open again.
    assert_eq!(
        opened(&resumed.see(Observation::Tick, t(DUE) + GRACE)),
        [AlertKind::MealNoRecord]
    );
    let replay = fed_body(1, FeedOutcome::Done, FeedReason::Done, CLIENT);
    assert_eq!(resumed.event(Some(now), replay, now + GRACE), []);
    let lapsed = resumed.see(Observation::Tick, now + SignedDuration::from_hours(24));
    assert_eq!(cleared(&lapsed), [AlertKind::ManualFeed]);
}

#[test]
fn a_clock_stepping_back_neither_lapses_nor_reopens_anything() {
    let mut watch = breakfast();
    watch.engine.preferences_mut().enable(AlertKind::ManualFeed);
    let now = t("2026-10-30T07:00:00Z");
    let manual = fed_body(1, FeedOutcome::Done, FeedReason::Done, CLIENT);
    watch.event(Some(now), manual, now);

    let back = t("2026-10-20T07:00:00Z");
    assert_eq!(watch.see(Observation::Tick, back), []);
    assert_eq!(watch.open_kinds(), [AlertKind::ManualFeed]);

    let mut watch = no_record();
    assert_eq!(watch.read_status(t("2026-10-30T08:10:00Z")), []);
    assert_eq!(watch.read_status(t(DUE) + GRACE), []);

    // A table read while the clock is back counts nothing before the
    // last point already ruled out.
    let mut watch = breakfast();
    watch.see(
        Observation::Plans(plans(2, &[(0, BREAKFAST)])),
        t(DUE) + GRACE,
    );
    watch.see(
        Observation::Plans(plans(3, &[(0, BREAKFAST)])),
        t("2026-10-30T07:00:00Z"),
    );
    assert_eq!(watch.read_status(t(DUE) + GRACE + GRACE), []);
}

#[test]
fn what_is_kept_stays_bounded() {
    let mut watch = breakfast();
    let mut now = t(DUE);

    for feed in 1..=60 {
        let at = now;
        watch.event(Some(at), done_by_plan(feed, 0), at);
        now += SignedDuration::from_hours(24);
    }

    let state = watch.engine.state();
    assert!(state.feeds.len() <= 3, "{}", state.feeds.len());
    assert!(state.seen.len() <= 9, "{}", state.seen.len());
}
