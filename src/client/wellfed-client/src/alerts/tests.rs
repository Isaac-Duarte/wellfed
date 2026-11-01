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
    /// Past the session's welcome, with the digest silenced, as a tick past
    /// its hour would open one; its own tests turn it back on.
    fn new() -> Self {
        let mut engine = Engine::default();
        engine.preferences_mut().disable(AlertKind::DailyDigest);
        let welcome = Observation::Welcome { epoch: EPOCH };
        engine.observe(welcome, t(START), &TimeZone::UTC);
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
    let levels = [
        (AlertKind::MealFailed, Level::Urgent),
        (AlertKind::MealUncertain, Level::Urgent),
        (AlertKind::MealMissed, Level::Urgent),
        (AlertKind::MealNoRecord, Level::Urgent),
        (AlertKind::NoClock, Level::Urgent),
        (AlertKind::CannotFeed, Level::Urgent),
        (AlertKind::FoodEmpty, Level::Urgent),
        (AlertKind::Unreachable, Level::Urgent),
        (AlertKind::FoodLow, Level::Soon),
        (AlertKind::DesiccantDue, Level::Soon),
        (AlertKind::PowerWasOut, Level::Soon),
        (AlertKind::BatteryBelowFloor, Level::Soon),
        (AlertKind::UpdateFailed, Level::Soon),
        (AlertKind::RolledBack, Level::Soon),
        (AlertKind::RepeatedRestarts, Level::Soon),
        (AlertKind::HistoryLost, Level::Soon),
        (AlertKind::ClientJoined, Level::Security),
        (AlertKind::ClientRevoked, Level::Security),
        (AlertKind::DailyDigest, Level::Optional),
        (AlertKind::MealFed, Level::Optional),
        (AlertKind::MealSoon, Level::Optional),
        (AlertKind::ManualFeed, Level::Optional),
    ];

    assert_eq!(levels.len(), 22);
    for (kind, level) in levels {
        assert_eq!(kind.level(), level, "{kind:?}");
    }
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
fn a_scheduled_feed_that_finished_replaces_the_news_and_leaves_a_failure_open() {
    let mut watch = breakfast();
    watch.engine.preferences_mut().enable(AlertKind::MealFed);
    watch.engine.preferences_mut().enable(AlertKind::ManualFeed);
    let at = t("2026-10-30T07:00:00Z");
    let manual = fed_body(1, FeedOutcome::Done, FeedReason::Done, CLIENT);
    let failed = fed_body(2, FeedOutcome::Failed, FeedReason::Jammed, CLIENT);
    watch.event(Some(at), manual, at);
    watch.event(Some(at), failed, at);

    let now = t(DUE);
    let changes = watch.event(Some(now), done_by_plan(3, 0), now);

    assert_eq!(cleared(&changes), [AlertKind::ManualFeed]);
    assert_eq!(opened(&changes), [AlertKind::MealFed]);
    assert_eq!(
        watch.open_kinds(),
        [AlertKind::MealFailed, AlertKind::MealFed]
    );
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
fn a_meal_problem_is_a_past_fact_no_feed_clears() {
    for (outcome, reason, kind) in [
        (
            FeedOutcome::Failed,
            FeedReason::Jammed,
            AlertKind::MealFailed,
        ),
        (
            FeedOutcome::Uncertain,
            FeedReason::Interrupted,
            AlertKind::MealUncertain,
        ),
    ] {
        let mut watch = breakfast();
        let now = t("2026-10-30T07:00:00Z");
        watch.event(Some(now), fed_body(1, outcome, reason, CLIENT), now);

        let later = now + SignedDuration::from_hours(72);
        assert!(cleared(&watch.see(Observation::Tick, later)).is_empty());
        let good = watch.event(Some(later), done_by_plan(2, 0), later);
        assert!(cleared(&good).is_empty());
        assert_eq!(watch.open_kinds(), [kind]);
    }

    let mut watch = breakfast();
    let now = t("2026-10-30T08:16:00Z");
    let missed = EventBody::Missed {
        plan: 0,
        due: t(DUE),
        reason: MissedReason::Late,
    };
    watch.event(Some(now), missed, now);
    let good = t("2026-10-31T08:00:30Z");
    assert!(cleared(&watch.event(Some(good), done_by_plan(1, 0), good)).is_empty());
    assert_eq!(watch.open_kinds(), [AlertKind::MealMissed]);
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
fn no_later_or_other_good_meal_closes_a_no_record() {
    let lunch = Plan {
        minute: 12 * 60,
        ..BREAKFAST
    };
    let mut watch = Watch::with_plans(plans(1, &[(0, BREAKFAST), (1, lunch)]), t(START));
    assert_eq!(
        opened(&watch.read_status(t(DUE) + GRACE)),
        [AlertKind::MealNoRecord]
    );

    let noon = t("2026-10-30T12:00:30Z");
    assert!(cleared(&watch.event(Some(noon), done_by_plan(2, 1), noon)).is_empty());
    let tomorrow = t("2026-10-31T08:00:30Z");
    assert!(cleared(&watch.event(Some(tomorrow), done_by_plan(3, 0), tomorrow)).is_empty());
    assert_eq!(watch.open_kinds(), [AlertKind::MealNoRecord]);
}

#[test]
fn the_next_days_failed_feed_is_not_yesterdays_record() {
    let mut watch = no_record();
    let tomorrow = t("2026-10-31T08:00:30Z");
    let failed = fed_body(
        1,
        FeedOutcome::Failed,
        FeedReason::Jammed,
        Source::Schedule { plan: 0 },
    );

    let changes = watch.event(Some(tomorrow), failed, tomorrow);

    assert!(cleared(&changes).is_empty());
    assert_eq!(opened(&changes), [AlertKind::MealFailed]);
}

#[test]
fn past_a_day_a_feed_answers_for_no_occurrence_even_with_no_plans_to_ask() {
    // A plan deleted since: the window alone bounds the record.
    let mut watch = no_record();
    let gone = Plan {
        days: Weekdays::from_bits(1),
        ..BREAKFAST
    };
    let read = t("2026-10-30T09:00:00Z");
    watch.see(Observation::Plans(plans(2, &[(0, gone)])), read);
    let late = t("2026-10-31T08:30:00Z");
    let failed = fed_body(
        1,
        FeedOutcome::Failed,
        FeedReason::Jammed,
        Source::Schedule { plan: 0 },
    );

    assert!(cleared(&watch.event(Some(late), failed, late)).is_empty());
    assert!(watch.open_kinds().contains(&AlertKind::MealNoRecord));
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
fn no_record_judges_on_a_status_exactly_as_old_as_fresh_and_no_older() {
    let mut watch = breakfast();
    watch.read_status(t("2026-10-30T08:09:59Z"));
    assert_eq!(watch.see(Observation::Tick, t(DUE) + GRACE), []);

    let mut watch = breakfast();
    watch.read_status(t("2026-10-30T08:10:00Z"));
    assert_eq!(
        opened(&watch.see(Observation::Tick, t(DUE) + GRACE)),
        [AlertKind::MealNoRecord]
    );
}

#[test]
fn a_status_from_before_a_drop_does_not_judge_after_the_reconnect() {
    let mut watch = breakfast();
    let fresh = t("2026-10-30T08:24:00Z");
    watch.read_status(fresh);

    watch.see(Observation::Link(Link::Failed), t("2026-10-30T08:24:20Z"));
    watch.see(
        Observation::Link(Link::Connected),
        t("2026-10-30T08:24:40Z"),
    );

    assert_eq!(watch.see(Observation::Tick, t(DUE) + GRACE), []);
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
    assert_eq!(
        opened(&changes),
        [AlertKind::HistoryLost, AlertKind::MealFailed]
    );
    assert_eq!(watch.engine.cursor(), Some((EPOCH + 1, 1)));
    assert_eq!(
        watch.open_kinds(),
        [
            AlertKind::MealFailed,
            AlertKind::HistoryLost,
            AlertKind::MealFailed
        ]
    );
}

#[test]
fn a_new_epoch_rules_out_meals_its_ring_no_longer_holds_and_says_so() {
    let mut watch = breakfast();
    let now = t("2026-10-30T07:00:00Z");
    let boot = EventBody::Boot {
        bank: 1,
        reset_flags: 0,
    };
    let seen = Event {
        epoch: EPOCH,
        seq: 1,
        at: Some(now),
        body: boot,
    };
    watch.see(Observation::Event(seen), now);

    let after_meal = t("2026-10-30T09:00:00Z");
    let wiped = Event {
        epoch: EPOCH + 1,
        at: Some(after_meal),
        ..seen
    };
    let changes = watch.see(Observation::Event(wiped), after_meal);

    let [Change::Opened(alert)] = changes[..] else {
        panic!("{changes:?}");
    };
    assert_eq!(alert.kind, AlertKind::HistoryLost);
    assert_eq!(alert.detail, Detail::Wiped);
    assert_eq!(alert.key.occurrence, Occurrence::Epoch { epoch: EPOCH + 1 });
    assert!(watch.engine.needs_plans());

    assert_eq!(see_status(&mut watch, &status(Some(1)), after_meal), []);
    let table = plans(1, &[(0, BREAKFAST)]);
    assert_eq!(watch.see(Observation::Plans(table), after_meal), []);
    assert_eq!(watch.open_kinds(), [AlertKind::HistoryLost]);
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

    assert!(!watch.engine.needs_plans());
    watch.event(Some(now), EventBody::Lost { count: 3 }, now);
    assert!(watch.engine.needs_plans());

    watch.see(Observation::Plans(plans(1, &[(0, BREAKFAST)])), now);
    assert!(!watch.engine.needs_plans());
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
fn acknowledging_a_no_record_closes_it_and_it_never_reopens() {
    let mut watch = no_record();
    let key = no_record_key(0, t(DUE));

    assert_eq!(watch.engine.acknowledge(key), Some(Change::Cleared(key)));
    assert_eq!(watch.engine.open().count(), 0);

    assert_eq!(watch.read_status(t("2026-10-30T08:40:00Z")), []);
    let now = t("2026-10-30T08:50:00Z");
    assert_eq!(watch.event(Some(now), done_by_plan(1, 0), now), []);
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

    let tracked = watch.engine.state().open[0].alert.key;
    assert_eq!(watch.engine.acknowledge(tracked), None);
    assert!(watch.engine.state().open.is_empty());
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

    // What the feeder recorded while it was stopped is unknown, so the
    // status from before does not judge a meal. A new one does, the grace
    // and the lapse go on from where they were, and what opened before the
    // restart does not open again.
    assert_eq!(resumed.see(Observation::Tick, t(DUE) + GRACE), []);
    assert_eq!(
        opened(&resumed.read_status(t(DUE) + GRACE)),
        [AlertKind::MealNoRecord]
    );
    let replay = Event {
        epoch: EPOCH,
        seq: 1,
        at: Some(now),
        body: manual,
    };
    assert_eq!(resumed.see(Observation::Event(replay), now + GRACE), []);
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
}

#[test]
fn a_clock_once_far_ahead_does_not_stop_no_record_for_good() {
    let mut watch = breakfast();
    let gap = Gap {
        epoch: EPOCH,
        from: 1,
        to: 40,
    };
    watch.see(Observation::Gap(gap), t("2036-01-01T00:00:00Z"));
    watch.seq = 40;

    let read = t("2026-10-30T07:00:00Z");
    watch.see(Observation::Plans(plans(1, &[(0, BREAKFAST)])), read);

    assert_eq!(
        opened(&watch.read_status(t(DUE) + GRACE)),
        [AlertKind::MealNoRecord]
    );
}

#[test]
fn a_clock_once_far_ahead_does_not_stop_the_digest_for_good() {
    let mut watch = quiet();
    watch
        .engine
        .preferences_mut()
        .enable(AlertKind::DailyDigest);
    watch.see(Observation::Tick, t("2036-01-01T09:00:00Z"));

    assert_eq!(watch.see(Observation::Tick, t("2026-10-30T09:00:00Z")), []);
    let changes = watch.see(Observation::Tick, t("2026-10-31T09:00:00Z"));
    assert!(opened(&changes).contains(&AlertKind::DailyDigest));
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
    assert!(state.seen.len() <= 18, "{}", state.seen.len());
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

/// `status` every ten minutes through `minutes`, as a live watcher reads it.
fn hold(watch: &mut Watch, status: &Status, minutes: std::ops::Range<i64>) -> Vec<Change> {
    minutes
        .step_by(10)
        .flat_map(|minute| see_status(watch, status, after(minute)))
        .collect()
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
    let baseline = Cursor {
        epoch: EPOCH,
        seq: 50,
    };
    assert_eq!(watch.engine.subscribe_after(), Some(baseline));
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
    assert_eq!(
        watch.engine.subscribe_after(),
        Some(Cursor {
            epoch: EPOCH,
            seq: 51
        })
    );
    assert_eq!(watch.engine.cursor(), Some((EPOCH, 51)));

    // A later status moves nothing: the cursor has taken over.
    see_status(&mut watch, &status(Some(90)), now);
    assert_eq!(watch.engine.cursor(), Some((EPOCH, 51)));
}

#[test]
fn a_status_takes_no_baseline_until_a_welcome_names_the_epoch() {
    let mut engine = Engine::default();
    let now = t(AFTER);
    let zone = TimeZone::UTC;

    engine.observe(Observation::Status(&status(Some(50))), now, &zone);
    assert_eq!(engine.subscribe_after(), None);

    engine.observe(Observation::Welcome { epoch: EPOCH }, now, &zone);
    assert_eq!(engine.subscribe_after(), None);

    engine.observe(Observation::Status(&status(Some(50))), now, &zone);
    assert_eq!(
        engine.subscribe_after(),
        Some(Cursor {
            epoch: EPOCH,
            seq: 50
        })
    );
}

#[test]
fn a_welcome_with_a_new_epoch_starts_the_read_over_in_that_epoch() {
    let mut watch = quiet();
    let welcome = Observation::Welcome { epoch: EPOCH + 1 };

    let changes = watch.see(welcome, after(0));
    assert_eq!(opened(&changes), [AlertKind::HistoryLost]);
    assert_eq!(watch.see(welcome, after(1)), []);
    assert_eq!(
        watch.engine.subscribe_after(),
        Some(Cursor {
            epoch: EPOCH + 1,
            seq: 0
        })
    );

    // Its events are read, and the same epoch raises nothing further.
    let event = Event {
        epoch: EPOCH + 1,
        seq: 1,
        at: None,
        body: fed_body(1, FeedOutcome::Failed, FeedReason::Jammed, CLIENT),
    };
    assert_eq!(
        opened(&watch.see(Observation::Event(event), after(2))),
        [AlertKind::MealFailed]
    );
}

#[test]
fn a_baseline_in_a_ring_wiped_since_does_not_swallow_the_new_rings_feeds() {
    let mut watch = Watch::new();
    let now = t(AFTER);
    see_status(&mut watch, &status(Some(50)), now);
    let failed = |feed| {
        fed_body(
            feed,
            FeedOutcome::Failed,
            FeedReason::Jammed,
            Source::Schedule { plan: 0 },
        )
    };
    let event = Event {
        epoch: EPOCH + 1,
        seq: 3,
        at: Some(now),
        body: failed(1),
    };

    let changes = watch.see(Observation::Event(event), now);
    assert_eq!(
        opened(&changes),
        [AlertKind::HistoryLost, AlertKind::MealFailed]
    );

    see_status(&mut watch, &status(Some(3)), now);
    let next = Event {
        seq: 4,
        body: failed(2),
        ..event
    };
    assert_eq!(
        opened(&watch.see(Observation::Event(next), now)),
        [AlertKind::MealFailed]
    );
    assert_eq!(
        watch.engine.subscribe_after(),
        Some(Cursor {
            epoch: EPOCH + 1,
            seq: 4
        })
    );
}

#[test]
fn a_status_counting_fewer_events_than_were_read_is_a_wipe() {
    let mut watch = quiet();
    let now = t(AFTER);
    let boot = EventBody::Boot {
        bank: 1,
        reset_flags: 0,
    };
    let event = Event {
        epoch: EPOCH,
        seq: 90,
        at: Some(now),
        body: boot,
    };
    watch.see(Observation::Event(event), now);

    let changes = see_status(&mut watch, &status(Some(3)), now);
    assert_eq!(opened(&changes), [AlertKind::HistoryLost]);
    assert!(watch.engine.needs_plans());
    assert_eq!(
        watch.engine.subscribe_after(),
        Some(Cursor {
            epoch: EPOCH,
            seq: 3
        })
    );

    // The new epoch, when it shows, is the same loss.
    let next = Event {
        epoch: EPOCH + 1,
        seq: 4,
        ..event
    };
    assert_eq!(watch.see(Observation::Event(next), now), []);
    assert_eq!(watch.engine.cursor(), Some((EPOCH + 1, 4)));
}

#[test]
fn a_feeder_with_no_events_has_no_history_to_skip() {
    let mut watch = quiet();
    assert_eq!(
        watch.engine.subscribe_after(),
        Some(Cursor {
            epoch: EPOCH,
            seq: 0
        })
    );

    let now = t(AFTER);
    let body = fed_body(1, FeedOutcome::Failed, FeedReason::Jammed, CLIENT);
    assert_eq!(
        opened(&watch.event(Some(now), body, now)),
        [AlertKind::MealFailed]
    );
}

#[test]
fn a_first_status_rules_out_what_fell_before_it() {
    let plans = plans(1, &[(0, BREAKFAST)]);
    let mut watch = Watch::new();
    watch.see(Observation::Plans(plans), t(START));

    assert_eq!(
        see_status(&mut watch, &status(Some(50)), t(DUE) + GRACE),
        []
    );

    // Its first look was 08:25, so tomorrow's is the first it can judge.
    assert_eq!(
        opened(&see_status(
            &mut watch,
            &status(Some(50)),
            t("2026-10-31T08:25:00Z")
        )),
        [AlertKind::MealNoRecord]
    );
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

    assert!(watch.engine.needs_plans());
    assert_eq!(watch.read_status(t("2026-10-31T08:25:00Z")), []);
}

#[test]
fn a_once_plan_the_feeder_gave_up_on_waits_for_the_table_too() {
    let once = Plan {
        days: Weekdays::ONCE,
        ..BREAKFAST
    };
    let mut watch = Watch::with_plans(plans(1, &[(0, once)]), t(START));
    let now = t("2026-10-30T08:16:00Z");
    let missed = EventBody::Missed {
        plan: 0,
        due: t(DUE),
        reason: MissedReason::Late,
    };
    watch.event(Some(now), missed, now);

    assert!(watch.engine.needs_plans());
    assert_eq!(watch.read_status(t("2026-10-31T08:25:00Z")), []);
}

#[test]
fn a_held_once_plan_holds_only_its_own_slot() {
    let once = Plan {
        days: Weekdays::ONCE,
        minute: 7 * 60,
        ..BREAKFAST
    };
    let mut watch = Watch::with_plans(plans(1, &[(0, BREAKFAST), (1, once)]), t(START));
    let ran = t("2026-10-30T07:00:30Z");
    watch.event(Some(ran), done_by_plan(1, 1), ran);

    assert_eq!(
        opened(&watch.read_status(t(DUE) + GRACE)),
        [AlertKind::MealNoRecord]
    );
}

#[test]
fn a_replayed_feed_does_not_spend_a_once_plan_armed_again_since() {
    let once = Plan {
        days: Weekdays::ONCE,
        ..BREAKFAST
    };
    let mut watch = Watch::with_plans(plans(1, &[(0, once)]), t(START));

    // The watcher was away: it ran on the 30th and was armed again.
    let back = t("2026-10-30T16:00:00Z");
    watch.see(Observation::Link(Link::Connected), back);
    watch.see(Observation::Plans(plans(6, &[(0, once)])), back);
    see_status(&mut watch, &status(Some(2)), back);
    watch.event(Some(t("2026-10-30T08:00:30Z")), done_by_plan(1, 0), back);
    let rearm = EventBody::Plans {
        generation: Generation(6),
        by: CLIENT,
    };
    watch.event(Some(back), rearm, back);

    // Held for a read rather than taken as spent; the read says it is not.
    assert!(watch.engine.needs_plans());
    let read = t("2026-10-30T16:10:00Z");
    watch.see(Observation::Plans(plans(6, &[(0, once)])), read);
    assert_eq!(
        opened(&watch.read_status(t("2026-10-31T08:25:00Z"))),
        [AlertKind::MealNoRecord]
    );
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

    assert_eq!(hold(&mut watch, &none, 0..60), []);
    assert_eq!(
        opened(&see_status(&mut watch, &none, after(60))),
        [AlertKind::NoClock]
    );
    assert_eq!(see_status(&mut watch, &none, after(70)), []);

    let changes = see_status(&mut watch, &status(None), after(71));
    assert_eq!(cleared(&changes), [AlertKind::NoClock]);
    assert_eq!(see_status(&mut watch, &none, after(72)), []);
}

#[test]
fn polls_far_apart_still_reach_every_status_timer() {
    for spacing in [16, 40] {
        let mut watch = quiet();
        let stuck = Status {
            clock: None,
            readiness: Some(Readiness::Blocked(Blocked::Fault)),
            ..on_bank(0)
        };
        see_status(&mut watch, &on_bank(1), after(0));

        let mut all = Vec::new();
        for poll in 0..=240 / spacing {
            all.extend(see_status(&mut watch, &stuck, after(1 + poll * spacing)));
        }

        let kinds = opened(&all);
        for kind in [
            AlertKind::NoClock,
            AlertKind::CannotFeed,
            AlertKind::RolledBack,
        ] {
            assert!(kinds.contains(&kind), "{spacing} minutes: {kind:?}");
        }
    }
}

#[test]
fn a_silence_past_three_thresholds_starts_a_status_timer_again() {
    let mut watch = quiet();
    let stuck = Status {
        clock: None,
        readiness: Some(Readiness::Blocked(Blocked::Fault)),
        ..on_bank(0)
    };
    see_status(&mut watch, &on_bank(1), after(0));
    see_status(&mut watch, &stuck, after(1));

    // Past three of every threshold, so this is a first reading of each.
    assert_eq!(see_status(&mut watch, &stuck, after(182)), []);

    let changes = hold(&mut watch, &stuck, 192..243);
    assert_eq!(
        opened(&changes),
        [
            AlertKind::CannotFeed,
            AlertKind::RolledBack,
            AlertKind::NoClock
        ]
    );
}

#[test]
fn a_silence_counts_against_each_timers_own_threshold() {
    let mut watch = quiet();
    let stuck = Status {
        clock: None,
        readiness: Some(Readiness::Blocked(Blocked::Fault)),
        ..status(None)
    };
    see_status(&mut watch, &stuck, after(0));

    // 91 minutes is past three of cannot feed's 30, inside three of no
    // clock's hour.
    assert_eq!(
        opened(&see_status(&mut watch, &stuck, after(91))),
        [AlertKind::NoClock]
    );
}

#[test]
fn a_silence_of_exactly_three_thresholds_does_not_restart() {
    let mut watch = quiet();
    let blocked = Status {
        readiness: Some(Readiness::Blocked(Blocked::Fault)),
        ..status(None)
    };
    see_status(&mut watch, &blocked, after(0));

    assert_eq!(
        opened(&see_status(&mut watch, &blocked, after(90))),
        [AlertKind::CannotFeed]
    );
}

#[test]
fn a_timer_restarts_at_once_when_a_status_shows_its_condition_gone() {
    let mut watch = quiet();
    let none = Status {
        clock: None,
        ..status(None)
    };
    see_status(&mut watch, &none, after(0));
    see_status(&mut watch, &status(None), after(40));

    assert_eq!(see_status(&mut watch, &none, after(80)), []);
    assert_eq!(see_status(&mut watch, &none, after(120)), []);
    assert_eq!(
        opened(&see_status(&mut watch, &none, after(140))),
        [AlertKind::NoClock]
    );
}

#[test]
fn cannot_feed_opens_past_its_threshold_with_the_reason_and_clears_on_ready() {
    let mut watch = quiet();
    let blocked = Status {
        readiness: Some(Readiness::Blocked(Blocked::Fault)),
        ..status(None)
    };

    assert_eq!(hold(&mut watch, &blocked, 0..30), []);
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
fn battery_below_floor_follows_the_feeders_verdict() {
    let mut watch = quiet();

    let flat = supply(Some(true), PACK, Some(true));
    assert_eq!(
        opened(&watch.event(None, flat, after(0))),
        [AlertKind::BatteryBelowFloor]
    );

    // No verdict is no verdict: it holds.
    let unjudged = supply(Some(true), Battery::Absent, None);
    assert!(cleared(&watch.event(None, unjudged, after(1))).is_empty());

    let good = supply(Some(true), PACK, Some(false));
    assert_eq!(
        cleared(&watch.event(None, good, after(2))),
        [AlertKind::BatteryBelowFloor]
    );
}

#[test]
fn a_pack_gone_stale_or_unread_does_not_clear_below_floor() {
    for battery in [Battery::Unread, Battery::Stale { age_ms: 90_000 }] {
        let mut watch = quiet();
        let flat = supply(Some(true), PACK, Some(true));
        watch.event(None, flat, after(0));

        let unsure = supply(Some(true), battery, Some(true));
        assert_eq!(watch.event(None, unsure, after(1)), []);
        assert_eq!(watch.open_kinds(), [AlertKind::BatteryBelowFloor]);
    }
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

    assert_eq!(hold(&mut watch, &on_bank(0), 0..30), []);
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
    let back = Status {
        last_event: Some(2),
        ..on_bank(1)
    };
    assert_eq!(see_status(&mut watch, &back, after(40)), []);

    // A feeder first seen on bank 0 fell from nowhere.
    let mut watch = Watch::new();
    see_status(&mut watch, &on_bank(0), after(0));
    assert_eq!(hold(&mut watch, &on_bank(0), 10..70), []);
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
    let opened_later = opened(&hold(&mut resumed, &none, 13..64));
    assert_eq!(opened_later, [AlertKind::NoClock]);
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

#[test]
fn a_meal_with_no_record_a_day_later_is_not_answered_by_the_next_days_feed() {
    let mut watch = breakfast();
    watch.see(Observation::Link(Link::Away), t("2026-10-30T07:00:00Z"));
    let back = t("2026-10-31T09:00:00Z");
    watch.see(Observation::Link(Link::Connected), back);
    assert_eq!(see_status(&mut watch, &status(Some(1)), back), []);

    // The last event of the backlog is what makes the meal judged.
    let fed = t("2026-10-31T08:10:00Z");
    let changes = watch.event(Some(fed), done_by_plan(1, 0), back);

    let [Change::Opened(alert)] = changes[..] else {
        panic!("{changes:?}");
    };
    assert_eq!(alert.key, no_record_key(0, t(DUE)));
}

#[test]
fn a_catch_up_read_leaves_a_failed_meal_open_past_the_good_one_after_it() {
    let lunch = Plan {
        minute: 12 * 60,
        ..BREAKFAST
    };
    for (outcome, reason, kind) in [
        (
            FeedOutcome::Uncertain,
            FeedReason::Interrupted,
            AlertKind::MealUncertain,
        ),
        (
            FeedOutcome::Failed,
            FeedReason::Jammed,
            AlertKind::MealFailed,
        ),
    ] {
        let mut watch = Watch::with_plans(plans(1, &[(0, BREAKFAST), (1, lunch)]), t(START));
        let back = t("2026-10-30T20:00:00Z");
        let by = Source::Schedule { plan: 0 };

        let mut all = watch.event(Some(t(DUE)), fed_body(1, outcome, reason, by), back);
        let noon = t("2026-10-30T12:00:30Z");
        all.extend(watch.event(Some(noon), done_by_plan(2, 1), back));

        assert_eq!(opened(&all), [kind]);
        assert_eq!(cleared(&all), []);
        assert_eq!(watch.open_kinds(), [kind]);
    }
}

#[test]
fn a_feed_stamped_in_the_future_answers_for_no_meal() {
    let mut watch = breakfast();
    let wrong = t("2027-10-30T08:00:30Z");
    watch.event(Some(wrong), done_by_plan(1, 0), t("2026-10-29T13:00:00Z"));

    assert!(watch.engine.state().feeds.is_empty());
    for day in [
        "2026-10-30T08:25:00Z",
        "2026-10-31T08:25:00Z",
        "2026-11-05T08:25:00Z",
    ] {
        let changes = watch.read_status(t(day));
        assert!(opened(&changes).contains(&AlertKind::MealNoRecord), "{day}");
    }
}

#[test]
fn a_feed_a_little_ahead_of_the_watcher_still_counts() {
    let mut watch = breakfast();
    let ahead = t("2026-10-30T08:02:00Z");
    watch.event(Some(ahead), done_by_plan(1, 0), t(DUE));

    assert_eq!(watch.read_status(t(DUE) + GRACE), []);
}

#[test]
fn a_gap_says_the_plans_must_be_read() {
    let mut watch = breakfast();
    let gap = Gap {
        epoch: EPOCH,
        from: 1,
        to: 40,
    };
    watch.see(Observation::Gap(gap), t("2026-10-30T07:00:00Z"));
    assert!(watch.engine.needs_plans());
}

/// A gap carries numbers, not times, so it rules out everything up to now.
/// This loses a meal the gap had no part in, which `HistoryLost` reports.
#[test]
fn a_gap_rules_out_todays_meal_though_it_cannot_say_when_it_fell() {
    let mut watch = breakfast();
    let back = t("2026-10-30T09:00:00Z");
    let gap = Gap {
        epoch: EPOCH,
        from: 1,
        to: 2,
    };
    watch.see(Observation::Link(Link::Connected), back);

    let changes = watch.see(Observation::Gap(gap), back);
    assert_eq!(opened(&changes), [AlertKind::HistoryLost]);

    watch.seq = 2;
    watch.see(Observation::Plans(plans(1, &[(0, BREAKFAST)])), back);
    assert_eq!(opened(&watch.read_status(back)), []);
}

#[test]
fn a_kind_turned_back_on_announces_a_condition_still_open() {
    let mut watch = quiet();
    watch.engine.preferences_mut().disable(AlertKind::FoodLow);
    let low = Status {
        food: FoodLevel::Low,
        ..status(None)
    };
    assert_eq!(see_status(&mut watch, &low, after(0)), []);

    watch.engine.preferences_mut().enable(AlertKind::FoodLow);
    assert_eq!(
        opened(&watch.see(Observation::Tick, after(1))),
        [AlertKind::FoodLow]
    );
    assert_eq!(watch.open_kinds(), [AlertKind::FoodLow]);
    assert_eq!(watch.see(Observation::Tick, after(2)), []);
}

#[test]
fn a_kind_turned_back_on_stays_silent_about_what_is_over() {
    let mut watch = quiet();
    watch
        .engine
        .preferences_mut()
        .disable(AlertKind::MealFailed);
    let now = t(AFTER);
    let failed = fed_body(1, FeedOutcome::Failed, FeedReason::Jammed, CLIENT);
    watch.event(Some(now), failed, now);

    watch.engine.preferences_mut().enable(AlertKind::MealFailed);
    assert_eq!(watch.see(Observation::Tick, after(1)), []);
}

fn with_fault(feed: u32, outcome: FeedOutcome, last_event: Option<u64>) -> Status {
    Status {
        feed_fault: Some(FeedFault {
            feed,
            outcome,
            reason: FeedReason::Jammed,
        }),
        ..status(last_event)
    }
}

#[test]
fn a_status_reports_a_fault_feed_whose_event_was_never_seen() {
    for (outcome, kind) in [
        (FeedOutcome::Failed, AlertKind::MealFailed),
        (FeedOutcome::Uncertain, AlertKind::MealUncertain),
    ] {
        let mut watch = quiet();
        let fault = with_fault(9, outcome, None);

        let changes = see_status(&mut watch, &fault, after(0));
        let [Change::Opened(alert)] = changes[..] else {
            panic!("{changes:?}");
        };
        assert_eq!(alert.kind, kind);
        assert!(matches!(alert.detail, Detail::Fault(_)));
        assert_eq!(
            alert.key.occurrence,
            Occurrence::Feed {
                epoch: EPOCH,
                feed: 9
            }
        );

        assert_eq!(see_status(&mut watch, &fault, after(1)), []);
    }
}

#[test]
fn a_fault_feed_already_read_opens_nothing_more() {
    let mut watch = quiet();
    let body = fed_body(9, FeedOutcome::Failed, FeedReason::Jammed, CLIENT);
    let now = t(AFTER);
    watch.event(Some(now), body, now);

    let fault = with_fault(9, FeedOutcome::Failed, Some(1));
    assert_eq!(see_status(&mut watch, &fault, after(1)), []);
}

#[test]
fn a_fault_feed_waits_for_the_backlog_so_its_fed_event_counts() {
    let mut watch = quiet();
    let now = t(AFTER);
    let behind = with_fault(9, FeedOutcome::Failed, Some(1));
    assert_eq!(see_status(&mut watch, &behind, now), []);

    let body = fed_body(9, FeedOutcome::Failed, FeedReason::Jammed, CLIENT);
    let changes = watch.event(Some(now), body, now);
    let [Change::Opened(alert)] = changes[..] else {
        panic!("{changes:?}");
    };
    assert!(matches!(alert.detail, Detail::Feed(_)));
}

#[test]
fn needs_plans_starts_true_and_a_read_answers_it() {
    let mut watch = Watch::new();
    assert!(watch.engine.needs_plans());

    watch.see(Observation::Plans(plans(1, &[])), t(AFTER));
    assert!(!watch.engine.needs_plans());
}

#[test]
fn a_fault_status_ahead_of_its_fed_leaves_the_feed_a_record() {
    let by = Source::Schedule { plan: 0 };
    let mut watch = breakfast();
    let seen = t("2026-10-30T08:01:00Z");
    let fault = with_fault(1, FeedOutcome::Failed, None);
    assert_eq!(
        opened(&see_status(&mut watch, &fault, seen)),
        [AlertKind::MealFailed]
    );

    let body = fed_body(1, FeedOutcome::Failed, FeedReason::Jammed, by);
    assert_eq!(watch.event(Some(seen), body, seen), []);

    assert_eq!(watch.engine.state().feeds.len(), 1);
    assert_eq!(watch.read_status(t(DUE) + GRACE), []);
    assert_eq!(watch.open_kinds(), [AlertKind::MealFailed]);
}

#[test]
fn a_fed_behind_a_fault_status_still_counts_its_portions() {
    let mut watch = quiet();
    let low = EventBody::Food {
        level: FoodLevel::Low,
    };
    watch.event(None, low, after(0));
    let fault = Status {
        food: FoodLevel::Low,
        ..with_fault(1, FeedOutcome::Failed, Some(1))
    };
    see_status(&mut watch, &fault, after(1));

    let body = EventBody::Fed {
        feed: 1,
        requested: 5,
        completed: 5,
        outcome: FeedOutcome::Failed,
        reason: FeedReason::Jammed,
        hopper: FoodLevel::Low,
        by: CLIENT,
    };

    assert_eq!(
        opened(&watch.event(None, body, after(2))),
        [AlertKind::FoodEmpty]
    );
}

#[test]
fn a_fed_behind_a_fault_status_still_spends_a_once_plan() {
    let once = Plan {
        days: Weekdays::ONCE,
        ..BREAKFAST
    };
    let by = Source::Schedule { plan: 0 };
    let mut watch = Watch::with_plans(plans(1, &[(0, once)]), t(START));
    let seen = t("2026-10-30T08:01:00Z");
    see_status(&mut watch, &with_fault(1, FeedOutcome::Failed, None), seen);

    let body = fed_body(1, FeedOutcome::Failed, FeedReason::Jammed, by);
    watch.event(Some(seen), body, seen);

    assert!(watch.engine.needs_plans());
}

#[test]
fn a_welcome_after_a_reported_wipe_still_says_the_history_went() {
    let mut watch = quiet();
    let now = t(AFTER);
    let boot = EventBody::Boot {
        bank: 1,
        reset_flags: 0,
    };
    let event = Event {
        epoch: EPOCH,
        seq: 10,
        at: Some(now),
        body: boot,
    };
    watch.see(Observation::Event(event), now);
    see_status(&mut watch, &status(Some(9)), now);

    let changes = watch.see(Observation::Welcome { epoch: EPOCH + 1 }, after(60));
    assert_eq!(opened(&changes), [AlertKind::HistoryLost]);
}

#[test]
fn a_reported_wipe_ends_with_the_next_event_read() {
    let mut watch = quiet();
    let now = t(AFTER);
    let boot = EventBody::Boot {
        bank: 1,
        reset_flags: 0,
    };
    let event = |epoch, seq| Event {
        epoch,
        seq,
        at: Some(now),
        body: boot,
    };
    watch.see(Observation::Event(event(EPOCH, 10)), now);
    see_status(&mut watch, &status(Some(9)), now);

    // The ring went on in its own epoch, so the next change of epoch is news.
    watch.see(Observation::Event(event(EPOCH, 11)), now);
    let changes = watch.see(Observation::Event(event(EPOCH + 1, 12)), after(60));

    assert_eq!(opened(&changes), [AlertKind::HistoryLost]);
}

#[test]
fn a_new_epoch_counting_from_below_a_reported_wipe_is_another_loss() {
    let mut watch = quiet();
    let now = t(AFTER);
    let boot = EventBody::Boot {
        bank: 1,
        reset_flags: 0,
    };
    let event = |epoch, seq| Event {
        epoch,
        seq,
        at: Some(now),
        body: boot,
    };
    watch.see(Observation::Event(event(EPOCH, 90)), now);
    see_status(&mut watch, &status(Some(30)), now);

    let changes = watch.see(Observation::Event(event(EPOCH + 1, 2)), after(60));

    assert_eq!(opened(&changes), [AlertKind::HistoryLost]);
}

#[test]
fn on_the_short_day_the_next_occurrence_ends_a_record_before_the_window_does() {
    let early = Plan {
        minute: 3 * 60 + 30,
        ..BREAKFAST
    };
    let mut watch = Watch::with_plans(plans(1, &[(0, early)]), t("2026-03-06T12:00:00Z"));
    watch.zone = TimeZone::posix("MST7MDT,M3.2.0,M11.1.0").unwrap();

    // 03:30 MST on March 7, and 23 hours on, 03:30 MDT on March 8.
    let due = t("2026-03-07T10:30:00Z");
    assert_eq!(
        opened(&watch.read_status(due + GRACE)),
        [AlertKind::MealNoRecord]
    );

    let next_days = t("2026-03-08T09:30:30Z");
    let failed = fed_body(
        1,
        FeedOutcome::Failed,
        FeedReason::Jammed,
        Source::Schedule { plan: 0 },
    );
    let changes = watch.event(Some(next_days), failed, next_days);

    assert!(cleared(&changes).is_empty(), "{changes:?}");
    assert!(watch.open_kinds().contains(&AlertKind::MealNoRecord));
}
