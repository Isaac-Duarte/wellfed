use wellfed_model::{FeedReason, UpdatePhase, Weekdays, WifiSource};

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
        clock: Some(t("2026-10-30T00:00:00Z")),
        power: None,
        locked: None,
        settings_generation: Generation(1),
        wifi_generation: Generation(1),
        last_event,
        sessions: 1,
        food: FoodLevel::Present,
        feed_fault: None,
        readiness: Some(Readiness::Ready),
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
    /// With the digest silenced, as a tick past its hour would open one;
    /// its own tests turn it back on.
    fn new() -> Self {
        let mut engine = Engine::default();
        engine.preferences_mut().disable(AlertKind::DailyDigest);
        Self {
            engine,
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
        let held = watch.see(Observation::Tick, t(DUE) + GRACE);
        assert!(!opened(&held).contains(&AlertKind::MealNoRecord));

        let back = t("2026-10-30T09:00:00Z");
        assert!(opened(&watch.see(Observation::Link(Link::Connected), back)).is_empty());
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
    let no_clock = Status {
        clock: None,
        ..status(None)
    };

    assert_eq!(
        opened(&watch.see(Observation::Status(&no_clock), t(DUE) + GRACE)),
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
    let [Change::Cleared(_), Change::Opened(alert)] = changes[..] else {
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
    let preferences = Preferences::default();
    assert!(preferences.enabled(AlertKind::ClientJoined));
    assert!(preferences.enabled(AlertKind::DailyDigest));

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
    let moved = Plan {
        minute: 7 * 60,
        ..BREAKFAST
    };
    watch.see(Observation::Plans(plans(2, &[(0, moved)])), t(DUE) + GRACE);
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

const AFTER: &str = "2026-10-30T10:00:00Z";

fn after(minutes: i64) -> Timestamp {
    t(AFTER) + SignedDuration::from_mins(minutes)
}

fn quiet() -> Watch {
    let mut watch = Watch::new();
    watch.see(Observation::Link(Link::Connected), t(AFTER));
    watch.read_status(t(AFTER));
    watch
}

fn see_status(watch: &mut Watch, status: &Status, now: Timestamp) -> Vec<Change> {
    watch.see(Observation::Status(status), now)
}

#[test]
fn a_new_engine_ignores_everything_until_a_status_and_then_only_what_follows_it() {
    let mut watch = Watch::new();
    let now = t(AFTER);
    let failed = |feed| fed_body(feed, FeedOutcome::Failed, FeedReason::Jammed, CLIENT);
    let event = |seq, body| Event {
        epoch: EPOCH,
        seq,
        at: Some(now),
        body,
    };

    assert_eq!(watch.engine.subscribe_after(), None);
    assert_eq!(watch.see(Observation::Event(event(3, failed(1))), now), []);
    assert_eq!(watch.engine.cursor(), None);

    see_status(&mut watch, &status(Some(50)), now);
    assert_eq!(watch.engine.subscribe_after(), Some(50));
    assert_eq!(watch.engine.cursor(), None);

    assert_eq!(watch.see(Observation::Event(event(49, failed(2))), now), []);
    assert_eq!(watch.see(Observation::Event(event(50, failed(3))), now), []);
    let gap = Gap {
        epoch: EPOCH,
        from: 1,
        to: 50,
    };
    assert_eq!(watch.see(Observation::Gap(gap), now), []);

    let changes = watch.see(Observation::Event(event(51, failed(4))), now);
    assert_eq!(opened(&changes), [AlertKind::MealFailed]);
    assert_eq!(watch.engine.subscribe_after(), Some(51));
    assert_eq!(watch.engine.cursor(), Some((EPOCH, 51)));

    // A later status moves nothing: the cursor has taken over.
    see_status(&mut watch, &status(Some(90)), now);
    assert_eq!(watch.engine.subscribe_after(), Some(51));
}

#[test]
fn a_feeder_with_no_events_has_no_history_to_skip() {
    let mut watch = quiet();
    assert_eq!(watch.engine.subscribe_after(), Some(0));

    let now = t(AFTER);
    let body = fed_body(1, FeedOutcome::Failed, FeedReason::Jammed, CLIENT);
    assert_eq!(
        opened(&watch.event(Some(now), body, now)),
        [AlertKind::MealFailed]
    );
}

#[test]
fn a_meal_is_judged_at_once_on_a_first_status_that_baselines_the_history() {
    let plans = plans(1, &[(0, BREAKFAST)]);
    let mut watch = Watch::new();
    watch.see(Observation::Plans(plans), t(START));

    let changes = see_status(&mut watch, &status(Some(50)), t(DUE) + GRACE);
    assert_eq!(opened(&changes), [AlertKind::MealNoRecord]);
}

#[test]
fn a_meal_problem_lapses_with_the_marks() {
    let mut watch = no_record();
    let now = t("2026-10-30T09:00:00Z");
    let failed = fed_body(1, FeedOutcome::Failed, FeedReason::Jammed, CLIENT);
    watch.event(Some(now), failed, now);
    let quiet_plans = plans(1, &[]);
    watch.see(Observation::Plans(quiet_plans), now);

    let days = |hours| now + SignedDuration::from_hours(hours);
    assert_eq!(watch.see(Observation::Tick, days(8 * 24 - 1)), []);
    let changes = watch.see(Observation::Tick, days(8 * 24));

    assert_eq!(
        cleared(&changes),
        [AlertKind::MealNoRecord, AlertKind::MealFailed]
    );
    assert!(watch.open_kinds().is_empty());
}

#[test]
fn a_done_after_an_uncertain_for_the_same_feed_changes_nothing() {
    let mut watch = breakfast();
    let now = t(DUE);
    let by = Source::Schedule { plan: 0 };
    let uncertain = fed_body(4, FeedOutcome::Uncertain, FeedReason::Interrupted, by);

    watch.event(Some(now), uncertain, now);
    assert_eq!(watch.event(Some(now), done_by_plan(4, 0), now), []);
    assert_eq!(watch.open_kinds(), [AlertKind::MealUncertain]);
}

#[test]
fn a_once_plan_taken_bumps_the_generation_and_hides_no_other_meal() {
    let once = Plan {
        days: Weekdays::ONCE,
        minute: 7 * 60,
        ..BREAKFAST
    };
    let mut watch = Watch::with_plans(plans(1, &[(0, BREAKFAST), (1, once)]), t(START));

    // It ran at 07:00, and the feeder disabled it with no event. A read
    // between then and the meal shows only a new generation.
    let ran = t("2026-10-30T07:00:30Z");
    watch.event(Some(ran), done_by_plan(1, 1), ran);
    let spent = Plan {
        enabled: false,
        ..once
    };
    let read = t("2026-10-30T07:30:00Z");
    watch.see(
        Observation::Plans(plans(2, &[(0, BREAKFAST), (1, spent)])),
        read,
    );

    assert_eq!(
        opened(&watch.read_status(t(DUE) + GRACE)),
        [AlertKind::MealNoRecord]
    );
}

#[test]
fn a_once_plan_taken_is_not_tomorrows_meal_before_the_table_is_read() {
    let once = Plan {
        days: Weekdays::ONCE,
        ..BREAKFAST
    };
    let mut watch = Watch::with_plans(plans(1, &[(0, once)]), t(START));
    let ran = t("2026-10-30T08:00:30Z");
    watch.event(Some(ran), done_by_plan(1, 0), ran);

    assert_eq!(watch.read_status(t("2026-10-31T08:25:00Z")), []);
}

#[test]
fn a_change_to_one_plan_leaves_the_others_counted() {
    let supper = Plan {
        minute: 18 * 60,
        ..BREAKFAST
    };
    let mut watch = Watch::with_plans(plans(1, &[(0, BREAKFAST)]), t(START));

    let read = t("2026-10-30T07:30:00Z");
    watch.see(
        Observation::Plans(plans(2, &[(0, BREAKFAST), (1, supper)])),
        read,
    );
    assert_eq!(
        opened(&watch.read_status(t(DUE) + GRACE)),
        [AlertKind::MealNoRecord]
    );

    // A wiped table with the same plans keeps them too.
    let read = t("2026-10-30T09:00:00Z");
    watch.see(
        Observation::Plans(plans(1, &[(0, BREAKFAST), (1, supper)])),
        read,
    );
    assert_eq!(
        opened(&watch.read_status(t("2026-10-30T18:25:00Z"))),
        [AlertKind::MealNoRecord]
    );
}

#[test]
fn no_clock_opens_past_its_threshold_and_clears_when_valid() {
    let mut watch = quiet();
    let none = Status {
        clock: None,
        ..status(None)
    };

    assert_eq!(see_status(&mut watch, &none, after(0)), []);
    assert_eq!(see_status(&mut watch, &none, after(59)), []);
    assert_eq!(
        opened(&see_status(&mut watch, &none, after(60))),
        [AlertKind::NoClock]
    );
    assert_eq!(see_status(&mut watch, &none, after(90)), []);

    let changes = see_status(&mut watch, &status(None), after(91));
    assert_eq!(cleared(&changes), [AlertKind::NoClock]);
    assert_eq!(see_status(&mut watch, &none, after(92)), []);
}

#[test]
fn cannot_feed_opens_past_its_threshold_with_the_reason_and_clears_on_ready() {
    let mut watch = quiet();
    let blocked = Status {
        readiness: Some(Readiness::Blocked(Blocked::Fault)),
        ..status(None)
    };

    assert_eq!(see_status(&mut watch, &blocked, after(0)), []);
    assert_eq!(see_status(&mut watch, &blocked, after(29)), []);
    let changes = see_status(&mut watch, &blocked, after(30));
    let [Change::Opened(alert)] = changes[..] else {
        panic!("{changes:?}");
    };
    assert_eq!(alert.kind, AlertKind::CannotFeed);
    assert_eq!(
        alert.detail,
        Detail::Blocked {
            reason: Blocked::Fault
        }
    );

    let unreported = Status {
        readiness: None,
        ..status(None)
    };
    assert_eq!(see_status(&mut watch, &unreported, after(31)), []);
    let changes = see_status(&mut watch, &status(None), after(32));
    assert_eq!(cleared(&changes), [AlertKind::CannotFeed]);
}

#[test]
fn food_low_opens_from_a_status_or_an_event_and_clears_on_present() {
    let mut watch = quiet();
    let low = Status {
        food: FoodLevel::Low,
        ..status(None)
    };
    assert_eq!(
        opened(&see_status(&mut watch, &low, after(0))),
        [AlertKind::FoodLow]
    );
    assert_eq!(see_status(&mut watch, &low, after(1)), []);
    let changes = see_status(&mut watch, &status(None), after(2));
    assert_eq!(cleared(&changes), [AlertKind::FoodLow]);

    let body = EventBody::Food {
        level: FoodLevel::Low,
    };
    assert_eq!(
        opened(&watch.event(None, body, after(3))),
        [AlertKind::FoodLow]
    );
    let present = EventBody::Food {
        level: FoodLevel::Present,
    };
    assert_eq!(
        cleared(&watch.event(None, present, after(4))),
        [AlertKind::FoodLow]
    );
}

#[test]
fn food_is_likely_out_after_five_portions_past_low() {
    let mut watch = quiet();
    let portions = |feed, completed, outcome| EventBody::Fed {
        feed,
        requested: 3,
        completed,
        outcome,
        reason: FeedReason::Done,
        hopper: FoodLevel::Low,
        by: Source::Schedule { plan: 0 },
    };
    let body = EventBody::Food {
        level: FoodLevel::Low,
    };

    // Before it read low does not count.
    watch.event(None, portions(1, 3, FeedOutcome::Done), after(0));
    watch.event(None, body, after(1));
    watch.event(None, portions(2, 3, FeedOutcome::Done), after(2));
    // An uncertain feed counts only what was confirmed, and only once.
    let uncertain = portions(3, 1, FeedOutcome::Uncertain);
    watch.event(None, uncertain, after(3));
    assert_eq!(watch.event(None, uncertain, after(4)), []);
    assert_eq!(watch.open_kinds().len(), 2);

    let changes = watch.event(None, portions(4, 1, FeedOutcome::Done), after(5));
    let empty = changes.iter().find_map(|change| match change {
        Change::Opened(alert) if alert.kind == AlertKind::FoodEmpty => Some(alert.detail),
        _ => None,
    });
    assert_eq!(empty, Some(Detail::Portions { confirmed: 5 }));

    let present = EventBody::Food {
        level: FoodLevel::Present,
    };
    let changes = watch.event(None, present, after(6));
    assert_eq!(
        cleared(&changes),
        [AlertKind::FoodLow, AlertKind::FoodEmpty]
    );
}

#[test]
fn desiccant_due_opens_and_clears_and_holds_with_no_clock() {
    let mut watch = quiet();
    let with = |desiccant| Status {
        desiccant,
        ..status(None)
    };
    let due = with(Desiccant::Remaining { days: 0, due: true });

    assert_eq!(
        opened(&see_status(&mut watch, &due, after(0))),
        [AlertKind::DesiccantDue]
    );
    assert_eq!(
        see_status(&mut watch, &with(Desiccant::Unknown), after(1)),
        []
    );
    assert_eq!(watch.open_kinds(), [AlertKind::DesiccantDue]);

    let fresh = with(Desiccant::Remaining {
        days: 30,
        due: false,
    });
    let changes = see_status(&mut watch, &fresh, after(2));
    assert_eq!(cleared(&changes), [AlertKind::DesiccantDue]);
}

fn supply(adapter: Option<bool>, battery: Battery, below_floor: Option<bool>) -> EventBody {
    EventBody::Supply {
        adapter,
        battery,
        below_floor,
    }
}

const PACK: Battery = Battery::Reading {
    mv: 4_000,
    percent: 50,
};

#[test]
fn power_was_out_opens_once_an_outage_and_clears_a_day_after_it_returned() {
    let mut watch = quiet();
    let out = supply(Some(false), PACK, Some(false));
    let back = supply(Some(true), PACK, Some(false));

    assert_eq!(
        opened(&watch.event(None, out, after(0))),
        [AlertKind::PowerWasOut]
    );
    assert_eq!(watch.event(None, out, after(1)), []);
    assert_eq!(watch.event(None, back, after(2)), []);

    let day = SignedDuration::from_hours(24);
    assert_eq!(
        watch.see(
            Observation::Tick,
            after(2) + day - SignedDuration::from_secs(1)
        ),
        []
    );
    let changes = watch.see(Observation::Tick, after(2) + day);
    assert_eq!(cleared(&changes), [AlertKind::PowerWasOut]);
}

#[test]
fn acknowledging_power_was_out_ends_it_and_only_a_new_outage_reopens() {
    let mut watch = quiet();
    let out = supply(Some(false), PACK, None);
    let back = supply(Some(true), PACK, None);
    let changes = watch.event(None, out, after(0));
    let [Change::Opened(alert)] = changes[..] else {
        panic!("{changes:?}");
    };

    assert_eq!(
        watch.engine.acknowledge(alert.key),
        Some(Change::Cleared(alert.key))
    );
    assert_eq!(watch.event(None, out, after(1)), []);

    watch.event(None, back, after(2));
    assert_eq!(
        opened(&watch.event(None, out, after(3))),
        [AlertKind::PowerWasOut]
    );
}

#[test]
fn battery_below_floor_needs_a_pack_or_no_adapter_and_clears_on_false() {
    let mut watch = quiet();

    // An empty holder on the adapter, as an older build judged it.
    let holder = supply(Some(true), Battery::Absent, Some(true));
    assert_eq!(watch.event(None, holder, after(0)), []);
    let unread = supply(Some(true), Battery::Unread, Some(true));
    assert_eq!(watch.event(None, unread, after(1)), []);

    let flat = supply(Some(true), PACK, Some(true));
    let changes = watch.event(None, flat, after(2));
    assert_eq!(opened(&changes), [AlertKind::BatteryBelowFloor]);
    let good = supply(Some(true), PACK, Some(false));
    assert_eq!(
        cleared(&watch.event(None, good, after(3))),
        [AlertKind::BatteryBelowFloor]
    );

    let unplugged = supply(Some(false), Battery::Unread, Some(true));
    let changes = watch.event(None, unplugged, after(4));
    assert!(opened(&changes).contains(&AlertKind::BatteryBelowFloor));

    // The verdict gone is no verdict: it holds.
    let unjudged = supply(Some(true), PACK, None);
    assert!(cleared(&watch.event(None, unjudged, after(5))).is_empty());
    assert_eq!(
        cleared(&watch.event(None, holder, after(6))),
        [AlertKind::BatteryBelowFloor]
    );
}

#[test]
fn a_status_reports_the_supply_as_an_event_does() {
    let mut watch = quiet();
    let unplugged = Status {
        supply: Some(Supply {
            adapter: Some(false),
            battery: PACK,
            below_floor: Some(true),
        }),
        ..status(None)
    };

    assert_eq!(
        opened(&see_status(&mut watch, &unplugged, after(0))),
        [AlertKind::PowerWasOut, AlertKind::BatteryBelowFloor]
    );
}

fn update(stopped: Option<UpdateStop>) -> UpdateView {
    UpdateView {
        phase: UpdatePhase::Idle,
        bank: 1,
        min_version: 1,
        digest: None,
        version: 0,
        image_len: 0,
        written: 0,
        wanted: Vec::new(),
        stopped,
    }
}

#[test]
fn update_failed_opens_on_a_stop_and_clears_when_there_is_none() {
    let mut watch = quiet();
    let hash = update(Some(UpdateStop::Hash));

    let changes = watch.see(Observation::Update(&hash), after(0));
    let [Change::Opened(alert)] = changes[..] else {
        panic!("{changes:?}");
    };
    assert_eq!(alert.kind, AlertKind::UpdateFailed);
    assert_eq!(alert.detail, Detail::Stopped(UpdateStop::Hash));
    assert_eq!(watch.see(Observation::Update(&hash), after(1)), []);

    let changes = watch.see(Observation::Update(&update(None)), after(2));
    assert_eq!(cleared(&changes), [AlertKind::UpdateFailed]);
}

#[test]
fn a_cancelled_install_is_not_a_failure() {
    let mut watch = quiet();
    let cancelled = update(Some(UpdateStop::Cancelled));

    assert_eq!(watch.see(Observation::Update(&cancelled), after(0)), []);

    watch.see(
        Observation::Update(&update(Some(UpdateStop::Write))),
        after(1),
    );
    let changes = watch.see(Observation::Update(&cancelled), after(2));
    assert_eq!(cleared(&changes), [AlertKind::UpdateFailed]);
}

fn on_bank(bank: u8) -> Status {
    Status {
        bank,
        ..status(None)
    }
}

#[test]
fn rolled_back_opens_after_bank_0_has_held_and_clears_on_bank_1() {
    let mut watch = quiet();

    assert_eq!(see_status(&mut watch, &on_bank(0), after(0)), []);
    assert_eq!(see_status(&mut watch, &on_bank(0), after(29)), []);
    assert_eq!(
        opened(&see_status(&mut watch, &on_bank(0), after(30))),
        [AlertKind::RolledBack]
    );

    let changes = see_status(&mut watch, &on_bank(1), after(31));
    assert_eq!(cleared(&changes), [AlertKind::RolledBack]);
}

#[test]
fn an_install_passing_through_bank_0_is_not_a_rollback() {
    let mut watch = quiet();
    let boot = |bank| EventBody::Boot {
        bank,
        reset_flags: 0,
    };

    watch.event(None, boot(0), after(0));
    watch.event(None, boot(1), after(20));
    assert_eq!(see_status(&mut watch, &on_bank(1), after(40)), []);

    // A feeder first seen on bank 0 fell from nowhere.
    let mut watch = Watch::new();
    see_status(&mut watch, &on_bank(0), after(0));
    assert_eq!(see_status(&mut watch, &on_bank(0), after(60)), []);
}

const WATCHDOG: u32 = 1 << 17;

fn restart(watch: &mut Watch, at: Option<Timestamp>, now: Timestamp) -> Vec<Change> {
    let body = EventBody::Boot {
        bank: 1,
        reset_flags: WATCHDOG,
    };
    watch.event(at, body, now)
}

#[test]
fn repeated_restarts_open_at_three_in_six_hours_and_clear_after_a_quiet_day() {
    let mut watch = quiet();
    assert_eq!(restart(&mut watch, None, after(0)), []);
    assert_eq!(restart(&mut watch, None, after(60)), []);
    assert_eq!(
        opened(&restart(&mut watch, None, after(120))),
        [AlertKind::RepeatedRestarts]
    );
    assert_eq!(restart(&mut watch, None, after(121)), []);

    let day = SignedDuration::from_hours(24);
    let changes = watch.see(Observation::Tick, after(121) + day);
    assert_eq!(cleared(&changes), [AlertKind::RepeatedRestarts]);
}

#[test]
fn restarts_spread_out_or_planned_do_not_count() {
    let mut watch = quiet();
    let hours = |n| after(0) + SignedDuration::from_hours(n);
    assert_eq!(restart(&mut watch, None, hours(0)), []);
    assert_eq!(restart(&mut watch, None, hours(4)), []);
    assert_eq!(restart(&mut watch, None, hours(8)), []);

    let mut watch = quiet();
    let power_on = EventBody::Boot {
        bank: 1,
        reset_flags: 0,
    };
    let deep_sleep = EventBody::Boot {
        bank: 1,
        reset_flags: 1 << 21,
    };
    for minutes in 0..4 {
        assert_eq!(watch.event(None, power_on, after(minutes)), []);
        assert_eq!(watch.event(None, deep_sleep, after(minutes)), []);
    }
}

#[test]
fn restarts_read_late_count_by_when_they_happened() {
    let mut watch = quiet();
    let now = after(600);
    let at = |minutes| Some(after(minutes));

    restart(&mut watch, at(0), now);
    restart(&mut watch, at(5), now);
    assert_eq!(
        opened(&restart(&mut watch, at(10), now)),
        [AlertKind::RepeatedRestarts]
    );
}

#[test]
fn a_client_joining_or_revoked_is_a_security_alert_that_lapses() {
    let client = Hex([9, 8, 7, 6]);
    let joined = EventBody::Enrolled {
        client,
        role: ClientRole::Member,
        path: EnrollPath::Invite,
    };
    let revoked = EventBody::Revoked { client, by: CLIENT };

    for (body, kind) in [
        (joined, AlertKind::ClientJoined),
        (revoked, AlertKind::ClientRevoked),
    ] {
        let mut watch = quiet();
        let changes = watch.event(None, body, after(0));
        let [Change::Opened(alert)] = changes[..] else {
            panic!("{changes:?}");
        };
        assert_eq!((alert.kind, alert.level), (kind, Level::Security));

        let day = SignedDuration::from_hours(24);
        assert_eq!(
            cleared(&watch.see(Observation::Tick, after(0) + day)),
            [kind]
        );
    }
}

#[test]
fn acknowledging_a_security_alert_closes_it() {
    let mut watch = quiet();
    let body = EventBody::Enrolled {
        client: Hex([9, 8, 7, 6]),
        role: ClientRole::Owner,
        path: EnrollPath::Press,
    };
    let changes = watch.event(None, body, after(0));
    let [Change::Opened(alert)] = changes[..] else {
        panic!("{changes:?}");
    };

    assert_eq!(
        watch.engine.acknowledge(alert.key),
        Some(Change::Cleared(alert.key))
    );
    assert_eq!(watch.engine.open().count(), 0);
}

#[test]
fn history_lost_opens_on_a_gap_and_on_lost_events_and_lapses() {
    let mut watch = quiet();
    let gap = Gap {
        epoch: EPOCH,
        from: 1,
        to: 40,
    };

    let changes = watch.see(Observation::Gap(gap), after(0));
    let [Change::Opened(alert)] = changes[..] else {
        panic!("{changes:?}");
    };
    assert_eq!(alert.kind, AlertKind::HistoryLost);
    assert_eq!(alert.level, Level::Soon);
    assert_eq!(alert.detail, Detail::Gap { from: 1, to: 40 });
    assert_eq!(watch.see(Observation::Gap(gap), after(1)), []);

    watch.seq = 40;
    let changes = watch.event(None, EventBody::Lost { count: 3 }, after(2));
    let [Change::Opened(alert)] = changes[..] else {
        panic!("{changes:?}");
    };
    assert_eq!(alert.detail, Detail::Lost { count: 3 });

    let day = SignedDuration::from_hours(24);
    let changes = watch.see(Observation::Tick, after(2) + day);
    assert_eq!(
        cleared(&changes),
        [AlertKind::HistoryLost, AlertKind::HistoryLost]
    );
}

#[test]
fn unreachable_opens_only_after_failing_for_its_threshold() {
    let mut watch = quiet();
    watch.see(Observation::Link(Link::Failed), after(0));
    assert_eq!(watch.see(Observation::Tick, after(9)), []);
    assert_eq!(
        opened(&watch.see(Observation::Link(Link::Failed), after(10))),
        [AlertKind::Unreachable]
    );

    let changes = watch.see(Observation::Link(Link::Connected), after(11));
    assert_eq!(cleared(&changes), [AlertKind::Unreachable]);
}

#[test]
fn being_away_pauses_and_resets_the_timer_and_never_opens_unreachable() {
    let mut watch = quiet();
    watch.see(Observation::Link(Link::Failed), after(0));
    watch.see(Observation::Link(Link::Away), after(9));
    assert_eq!(watch.see(Observation::Tick, after(600)), []);

    watch.see(Observation::Link(Link::Failed), after(601));
    assert_eq!(watch.see(Observation::Tick, after(610)), []);
    assert_eq!(
        opened(&watch.see(Observation::Tick, after(611))),
        [AlertKind::Unreachable]
    );
}

#[test]
fn an_open_unreachable_stays_while_away_and_a_status_clears_it() {
    let mut watch = quiet();
    watch.see(Observation::Link(Link::Failed), after(0));
    watch.see(Observation::Tick, after(10));

    assert_eq!(watch.see(Observation::Link(Link::Away), after(11)), []);
    assert_eq!(watch.see(Observation::Tick, after(500)), []);
    assert_eq!(watch.open_kinds(), [AlertKind::Unreachable]);

    let changes = watch.read_status(after(501));
    assert_eq!(cleared(&changes), [AlertKind::Unreachable]);
}

#[test]
fn an_acknowledged_condition_stays_open_and_does_not_open_again() {
    let mut watch = quiet();
    watch.see(Observation::Link(Link::Failed), after(0));
    let changes = watch.see(Observation::Tick, after(10));
    let [Change::Opened(alert)] = changes[..] else {
        panic!("{changes:?}");
    };

    assert_eq!(watch.engine.acknowledge(alert.key), None);
    assert_eq!(watch.see(Observation::Tick, after(20)), []);
    assert!(watch.engine.open().next().unwrap().acknowledged);
}

#[test]
fn a_digest_comes_once_a_local_day_with_what_happened_since_the_last() {
    let mut watch = quiet();
    watch
        .engine
        .preferences_mut()
        .enable(AlertKind::DailyDigest);
    let day_one = t("2026-10-30T09:00:00Z");
    assert_eq!(
        opened(&watch.see(Observation::Tick, day_one)),
        [AlertKind::DailyDigest]
    );
    assert_eq!(watch.see(Observation::Tick, after(0)), []);

    let at = t("2026-10-30T12:00:00Z");
    watch.event(Some(at), done_by_plan(1, 0), at);
    let failed = fed_body(2, FeedOutcome::Failed, FeedReason::Jammed, CLIENT);
    watch.event(Some(at), failed, at);

    // Not before the hour.
    assert_eq!(watch.see(Observation::Tick, t("2026-10-31T07:59:59Z")), []);

    let changes = watch.see(Observation::Tick, t("2026-10-31T08:00:00Z"));
    assert_eq!(cleared(&changes), [AlertKind::DailyDigest]);
    let [_, Change::Opened(alert)] = changes[..] else {
        panic!("{changes:?}");
    };
    assert_eq!(
        alert.detail,
        Detail::Digest {
            fed: 1,
            problems: 1
        }
    );

    // A clock stepping back to yesterday does not send it again.
    assert_eq!(watch.see(Observation::Tick, t("2026-10-30T09:00:00Z")), []);
}

#[test]
fn only_a_tick_brings_the_digest() {
    let mut watch = quiet();
    watch
        .engine
        .preferences_mut()
        .enable(AlertKind::DailyDigest);

    assert_eq!(watch.read_status(t("2026-10-30T09:00:00Z")), []);
}

#[test]
fn timers_resume_from_stored_state() {
    let mut watch = quiet();
    watch.see(Observation::Link(Link::Failed), after(0));
    let none = Status {
        clock: None,
        ..status(None)
    };
    see_status(&mut watch, &none, after(1));
    watch.see(Observation::Link(Link::Failed), after(2));

    let json = serde_json::to_string(watch.engine.state()).unwrap();
    let state: State = serde_json::from_str(&json).unwrap();
    assert_eq!(&state, watch.engine.state());

    let mut resumed = Watch {
        engine: Engine::resume(state, Thresholds::default()),
        zone: TimeZone::UTC,
        seq: 0,
    };
    assert_eq!(
        opened(&resumed.see(Observation::Tick, after(12))),
        [AlertKind::Unreachable]
    );
    assert_eq!(
        opened(&see_status(&mut resumed, &none, after(61))),
        [AlertKind::NoClock]
    );
}

#[test]
fn a_clock_stepping_back_does_not_trip_a_timer() {
    let mut watch = quiet();
    let none = Status {
        clock: None,
        ..status(None)
    };
    see_status(&mut watch, &none, after(0));

    assert_eq!(see_status(&mut watch, &none, t("2026-10-01T00:00:00Z")), []);
    assert_eq!(see_status(&mut watch, &none, after(59)), []);
}

#[test]
fn restarts_stay_bounded() {
    let mut watch = quiet();
    for minutes in 0..50 {
        restart(&mut watch, None, after(minutes));
    }

    assert_eq!(watch.engine.state().restarts.len(), 3);
    assert_eq!(watch.open_kinds(), [AlertKind::RepeatedRestarts]);
}
