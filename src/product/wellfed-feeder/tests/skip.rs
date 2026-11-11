//! A client's skip of today's meal and its undo, checked against the plans
//! and the zone's offsets (`feeder::skip`), then applied as the writer
//! applies them (`Plans::changed`). Simulation: the occurrences are judged
//! by the same missed-meal rule the schedule runs, at fixed instants
//! across both of 2026's daylight changes in Central Europe.

use wellfed_core::clock::wall::{Occurrence, occurrence};
use wellfed_device_api::mailbox::{Claim, MealSkip, PLANS, Plan, PlansChange, PlansView, Weekdays};
use wellfed_feeder::feeder::skip::change;
use wellfed_feeder::store::plans::Plans;
use wellfed_proto::api::Refusal;

/// 2026-03-29 01:00 UTC, when Central Europe moves from +1 h to +2 h.
const SPRING_MS: u64 = 1_774_746_000_000;
/// 2026-10-25 01:00 UTC, when it moves back.
const AUTUMN_MS: u64 = 1_792_890_000_000;
const MINUTE_MS: u64 = 60_000;
const HOUR_MS: u64 = 60 * MINUTE_MS;
const REVISION: u32 = 7;

fn europe(unix_ms: u64) -> Option<i32> {
    Some(if (SPRING_MS..AUTUMN_MS).contains(&unix_ms) {
        7_200
    } else {
        3_600
    })
}

fn plan(minute: u16, days: Weekdays) -> Plan {
    Plan {
        minute,
        portions: 2,
        days,
        enabled: true,
        call: 0,
    }
}

/// `plan` alone in slot 0, with mark `mark_s`.
fn view(plan: Plan, mark_s: u32) -> PlansView {
    let mut table = [None; PLANS];
    table[0] = Some(plan);
    let mut marks = [0; PLANS];
    marks[0] = mark_s;
    PlansView {
        table,
        marks,
        skips: 0,
        fed: 0,
        claim: None,
        revision: REVISION,
        replaced: 0,
        writable: true,
    }
}

/// The view once the writer applies `change`.
fn apply(view: &PlansView, change: &PlansChange) -> PlansView {
    let plans = Plans {
        table: view.table,
        marks: view.marks,
        skips: view.skips,
        fed: view.fed,
        claim: view.claim,
        revision: view.revision,
    };
    plans
        .changed(change)
        .expect("the writer takes it")
        .view(true)
}

fn seconds(unix_ms: u64) -> u32 {
    u32::try_from(unix_ms / 1000).unwrap()
}

/// How the schedule judges slot 0's occurrence `due_ms` at `wall_ms`.
fn judged(view: &PlansView, due_ms: u64, wall_ms: u64) -> Occurrence {
    let mark = view.mark(0).map(|mark| u64::from(mark) * 1000);
    occurrence(due_ms, mark, wall_ms)
}

/// Skip `due_ms` at `now_ms` with the mark at the plan's occurrence before
/// it, `before_ms`; check it never runs; undo it; check the mark is above
/// where it was, below the meal, and the meal runs when due.
fn skip_and_undo(plan: Plan, before_ms: u64, due_ms: u64, now_ms: u64) {
    let due_s = seconds(due_ms);
    let start = view(plan, seconds(before_ms));

    let skip = change(&start, 0, due_s, MealSkip::Skip, now_ms, europe).unwrap();
    assert_eq!(
        skip,
        PlansChange::Skip {
            plan: 0,
            due_s,
            revision: REVISION
        }
    );
    let skipped = apply(&start, &skip);
    assert_eq!(skipped.marks[0], due_s);
    for wall_ms in [due_ms, due_ms + MINUTE_MS, due_ms + 20 * MINUTE_MS] {
        assert_eq!(judged(&skipped, due_ms, wall_ms), Occurrence::Evaluated);
    }

    let unskip = change(&skipped, 0, due_s, MealSkip::Unskip, now_ms, europe).unwrap();
    assert_eq!(
        unskip,
        PlansChange::Unskip {
            plan: 0,
            due_s,
            revision: REVISION
        }
    );
    let undone = apply(&skipped, &unskip);
    assert_eq!(undone.marks[0], due_s - 1);
    assert!(
        undone.marks[0] >= start.marks[0],
        "never below the mark before"
    );
    // No occurrence between: the one before the meal stays spent.
    assert_eq!(judged(&undone, before_ms, now_ms), Occurrence::Evaluated);
    assert_eq!(judged(&undone, due_ms, due_ms - 1), Occurrence::Later);
    assert_eq!(judged(&undone, due_ms, due_ms), Occurrence::Run);
}

#[test]
fn a_skipped_meal_never_runs_and_an_undone_one_runs_on_a_plain_day() {
    // Wednesday 2026-11-04, +1 h: 18:00 local is 17:00 UTC.
    let due_ms = 1_793_811_600_000;
    skip_and_undo(
        plan(18 * 60, Weekdays::EVERY_DAY),
        due_ms - 24 * HOUR_MS,
        due_ms,
        due_ms - 5 * HOUR_MS,
    );
}

#[test]
fn the_spring_change_leaves_23_hours_between_the_meals_and_a_skip_and_its_undo_hold() {
    // 08:00 local: 07:00 UTC on the Saturday, 06:00 UTC on the Sunday.
    let due_ms = 1_774_764_000_000;
    let before_ms = 1_774_681_200_000;
    assert_eq!(due_ms - before_ms, 23 * HOUR_MS);
    skip_and_undo(
        plan(8 * 60, Weekdays::EVERY_DAY),
        before_ms,
        due_ms,
        due_ms - HOUR_MS,
    );

    // 02:30 local, which the change skips: the meal runs at 03:30 local
    // (01:30 UTC), as the schedule runs it.
    let due_ms = SPRING_MS + 30 * MINUTE_MS;
    skip_and_undo(
        plan(2 * 60 + 30, Weekdays::EVERY_DAY),
        due_ms - 24 * HOUR_MS,
        due_ms,
        SPRING_MS + 10 * MINUTE_MS,
    );
}

#[test]
fn the_autumn_change_leaves_25_hours_between_the_meals_and_a_repeated_minute_skips_its_first_pass()
{
    // 08:00 local: 06:00 UTC on the Saturday, 07:00 UTC on the Sunday.
    let due_ms = 1_792_911_600_000;
    let before_ms = 1_792_821_600_000;
    assert_eq!(due_ms - before_ms, 25 * HOUR_MS);
    skip_and_undo(
        plan(8 * 60, Weekdays::EVERY_DAY),
        before_ms,
        due_ms,
        due_ms - HOUR_MS,
    );

    // 02:30 local comes twice; the meal is its first pass, 00:30 UTC.
    let due_ms = AUTUMN_MS - 30 * MINUTE_MS;
    let repeated = plan(2 * 60 + 30, Weekdays::EVERY_DAY);
    skip_and_undo(
        repeated,
        due_ms - 24 * HOUR_MS,
        due_ms,
        due_ms - 30 * MINUTE_MS,
    );
    // The second pass is no occurrence, so it is not a meal to skip.
    let start = view(repeated, seconds(due_ms));
    let second = seconds(due_ms + HOUR_MS);
    let refused = change(&start, 0, second, MealSkip::Skip, due_ms + 1, europe);
    assert_eq!(refused, Err(Refusal::Invalid));
}

#[test]
fn a_weekday_plan_skips_only_today_s_meal_and_its_undo_feeds_it() {
    // Mondays and Wednesdays at 18:00 local; Wednesday 2026-11-04 17:00 UTC.
    let weekdays = plan(18 * 60, Weekdays(0b101));
    let wednesday_ms = 1_793_811_600_000;
    let monday_ms = wednesday_ms - 48 * HOUR_MS;
    skip_and_undo(
        weekdays,
        monday_ms,
        wednesday_ms,
        wednesday_ms - 5 * HOUR_MS,
    );

    // On the Tuesday, Wednesday's meal is the next but not today's.
    let tuesday_noon = wednesday_ms - 29 * HOUR_MS;
    let start = view(weekdays, seconds(monday_ms));
    let skip = change(
        &start,
        0,
        seconds(wednesday_ms),
        MealSkip::Skip,
        tuesday_noon,
        europe,
    );
    assert_eq!(skip, Err(Refusal::Invalid));
}

#[test]
fn a_plan_saved_without_a_clock_can_be_skipped_and_its_undo_lets_the_meal_run() {
    let due_ms = 1_793_811_600_000;
    let now_ms = due_ms - 5 * HOUR_MS;
    let start = view(plan(18 * 60, Weekdays::EVERY_DAY), 0);
    assert_eq!(start.mark(0), None);

    let skip = change(&start, 0, seconds(due_ms), MealSkip::Skip, now_ms, europe).unwrap();
    let skipped = apply(&start, &skip);
    assert_eq!(judged(&skipped, due_ms, due_ms), Occurrence::Evaluated);

    // The undo leaves a mark, so this meal runs, as it would once the
    // schedule passed the one before, unreported.
    let unskip = change(
        &skipped,
        0,
        seconds(due_ms),
        MealSkip::Unskip,
        now_ms,
        europe,
    );
    let undone = apply(&skipped, &unskip.unwrap());
    assert_eq!(undone.marks[0], seconds(due_ms) - 1);
    assert_eq!(judged(&undone, due_ms, due_ms), Occurrence::Run);
}

#[test]
fn a_plan_that_runs_once_is_disabled_by_its_skip_and_enabled_by_its_undo() {
    let due_ms = 1_793_811_600_000;
    let now_ms = due_ms - 5 * HOUR_MS;
    let once = plan(18 * 60, Weekdays::ONCE);
    // Saved this morning: its mark is the save.
    let start = view(once, seconds(now_ms - HOUR_MS));

    let skip = change(&start, 0, seconds(due_ms), MealSkip::Skip, now_ms, europe).unwrap();
    let skipped = apply(&start, &skip);
    assert!(!skipped.table[0].unwrap().enabled);
    assert_eq!(skipped.revision, REVISION + 1);

    let unskip = change(
        &skipped,
        0,
        seconds(due_ms),
        MealSkip::Unskip,
        now_ms,
        europe,
    );
    let undone = apply(&skipped, &unskip.unwrap());
    assert!(undone.table[0].unwrap().enabled);
    assert_eq!(judged(&undone, due_ms, due_ms), Occurrence::Run);
}

#[test]
fn an_undo_once_the_meal_is_due_is_refused_and_the_meal_stays_skipped() {
    let due_ms = 1_793_811_600_000;
    let start = view(
        plan(18 * 60, Weekdays::EVERY_DAY),
        seconds(due_ms - 24 * HOUR_MS),
    );
    let skip = change(
        &start,
        0,
        seconds(due_ms),
        MealSkip::Skip,
        due_ms - HOUR_MS,
        europe,
    );
    let skipped = apply(&start, &skip.unwrap());

    for now_ms in [due_ms, due_ms + 1, due_ms + 10 * MINUTE_MS] {
        let undo = change(
            &skipped,
            0,
            seconds(due_ms),
            MealSkip::Unskip,
            now_ms,
            europe,
        );
        assert_eq!(undo, Err(Refusal::Invalid), "at {now_ms}");
    }
    // A skip of a meal already due is refused the same way.
    let late = change(&start, 0, seconds(due_ms), MealSkip::Skip, due_ms, europe);
    assert_eq!(late, Err(Refusal::Invalid));
}

#[test]
fn only_the_plan_s_own_next_meal_can_be_skipped_and_only_once() {
    let due_ms = 1_793_811_600_000;
    let now_ms = due_ms - 5 * HOUR_MS;
    let every_day = plan(18 * 60, Weekdays::EVERY_DAY);
    let start = view(every_day, seconds(due_ms - 24 * HOUR_MS));
    let skip =
        |view: &PlansView, slot, due_s| change(view, slot, due_s, MealSkip::Skip, now_ms, europe);

    assert_eq!(skip(&start, 1, seconds(due_ms)), Err(Refusal::NotFound));
    assert_eq!(skip(&start, 10, seconds(due_ms)), Err(Refusal::NotFound));
    let off = view(
        Plan {
            enabled: false,
            ..every_day
        },
        seconds(due_ms - 24 * HOUR_MS),
    );
    assert_eq!(skip(&off, 0, seconds(due_ms)), Err(Refusal::NotFound));
    assert_eq!(skip(&start, 0, seconds(due_ms) + 60), Err(Refusal::Invalid));
    let tomorrow = seconds(due_ms + 24 * HOUR_MS);
    assert_eq!(skip(&start, 0, tomorrow), Err(Refusal::Invalid));

    let skipped = apply(&start, &skip(&start, 0, seconds(due_ms)).unwrap());
    assert_eq!(skip(&skipped, 0, seconds(due_ms)), Err(Refusal::Invalid));

    // An undo of a meal that was never skipped.
    let undo = change(&start, 0, seconds(due_ms), MealSkip::Unskip, now_ms, europe);
    assert_eq!(undo, Err(Refusal::Invalid));
}

#[test]
fn without_a_zone_offset_nothing_is_skipped() {
    let due_ms = 1_793_811_600_000;
    let start = view(
        plan(18 * 60, Weekdays::EVERY_DAY),
        seconds(due_ms - 24 * HOUR_MS),
    );
    for skip in [MealSkip::Skip, MealSkip::Unskip] {
        let refused = change(&start, 0, seconds(due_ms), skip, due_ms - HOUR_MS, |_| None);
        assert_eq!(refused, Err(Refusal::NoClock));
    }
}

#[test]
fn a_meal_before_the_skipped_one_still_unsettled_holds_the_skip() {
    // Yesterday's meal is past the mark: the schedule may still run it, or
    // report it missed, so a skip now would spend it unseen.
    let due_ms = 1_793_811_600_000;
    let now_ms = due_ms - 5 * HOUR_MS;
    let yesterday_ms = due_ms - 24 * HOUR_MS;
    let every_day = plan(18 * 60, Weekdays::EVERY_DAY);
    let start = view(every_day, seconds(yesterday_ms - 24 * HOUR_MS));
    let skip = change(&start, 0, seconds(due_ms), MealSkip::Skip, now_ms, europe);
    assert_eq!(skip, Err(Refusal::Busy));

    // Its claim settles it.
    let claimed = PlansView {
        claim: Some(Claim {
            plan: 0,
            due_s: seconds(yesterday_ms),
            high_water: 4,
            disabled: false,
        }),
        ..start
    };
    let skip = change(&claimed, 0, seconds(due_ms), MealSkip::Skip, now_ms, europe);
    assert!(skip.is_ok());
}

#[test]
fn an_undo_never_lowers_a_mark_a_save_left_above_the_meal_before() {
    // Saved this morning, after yesterday's meal: the mark is the save.
    let due_ms = 1_793_811_600_000;
    let now_ms = due_ms - 2 * HOUR_MS;
    let saved_s = seconds(due_ms - 5 * HOUR_MS);
    let start = view(plan(18 * 60, Weekdays::EVERY_DAY), saved_s);

    let skip = change(&start, 0, seconds(due_ms), MealSkip::Skip, now_ms, europe);
    let skipped = apply(&start, &skip.unwrap());
    let unskip = change(
        &skipped,
        0,
        seconds(due_ms),
        MealSkip::Unskip,
        now_ms,
        europe,
    );
    let undone = apply(&skipped, &unskip.unwrap());
    assert!(undone.marks[0] > saved_s);
    assert_eq!(judged(&undone, due_ms, due_ms), Occurrence::Run);
}

#[test]
fn an_undo_after_the_offsets_moved_the_plan_s_next_meal_is_refused() {
    let due_ms = 1_793_811_600_000;
    let now_ms = due_ms - 5 * HOUR_MS;
    let start = view(
        plan(18 * 60, Weekdays::EVERY_DAY),
        seconds(due_ms - 24 * HOUR_MS),
    );
    let skip = change(&start, 0, seconds(due_ms), MealSkip::Skip, now_ms, europe);
    let skipped = apply(&start, &skip.unwrap());

    // A zone two hours further east: 18:00 local now comes two hours
    // sooner, so the meal skipped is no longer the plan's next.
    let moved = |unix_ms| europe(unix_ms).map(|offset| offset + 7_200);
    let undo = change(
        &skipped,
        0,
        seconds(due_ms),
        MealSkip::Unskip,
        now_ms,
        moved,
    );
    assert_eq!(undo, Err(Refusal::Invalid));
}

#[test]
fn an_undo_of_a_meal_that_ran_is_refused_after_the_clock_moves_back() {
    // Every day at 18:00: today's meal ran (claimed, then settled into the
    // mark), and the clock then moved back five minutes before it.
    let due_ms = 1_793_811_600_000;
    let every_day = plan(18 * 60, Weekdays::EVERY_DAY);
    let start = view(every_day, seconds(due_ms - 24 * HOUR_MS));
    let held = Claim {
        plan: 0,
        due_s: seconds(due_ms),
        high_water: 4,
        disabled: false,
    };
    let claimed = apply(
        &start,
        &PlansChange::Claim {
            claim: held,
            revision: REVISION,
        },
    );
    let ran = apply(&claimed, &PlansChange::Settle { claim: held });
    assert_eq!(ran.marks[0], seconds(due_ms));

    let back_ms = due_ms - 5 * MINUTE_MS;
    for view in [&claimed, &ran] {
        let undo = change(view, 0, seconds(due_ms), MealSkip::Unskip, back_ms, europe);
        assert_eq!(undo, Err(Refusal::Invalid));
        assert_eq!(judged(view, due_ms, due_ms), Occurrence::Evaluated);
    }
}

#[test]
fn an_undo_of_a_missed_meal_is_refused_after_the_clock_moves_back() {
    let due_ms = 1_793_811_600_000;
    let start = view(
        plan(18 * 60, Weekdays::EVERY_DAY),
        seconds(due_ms - 24 * HOUR_MS),
    );
    let passed = apply(
        &start,
        &PlansChange::Pass {
            plan: 0,
            due_s: seconds(due_ms),
            revision: REVISION,
        },
    );

    let back_ms = due_ms - 5 * MINUTE_MS;
    let undo = change(
        &passed,
        0,
        seconds(due_ms),
        MealSkip::Unskip,
        back_ms,
        europe,
    );
    assert_eq!(undo, Err(Refusal::Invalid));
}

#[test]
fn an_undo_of_a_plan_that_ran_once_is_refused_after_the_clock_moves_back() {
    // It ran, and disabled itself, as a skip would have.
    let due_ms = 1_793_811_600_000;
    let start = view(plan(18 * 60, Weekdays::ONCE), seconds(due_ms - HOUR_MS));
    let held = Claim {
        plan: 0,
        due_s: seconds(due_ms),
        high_water: 4,
        disabled: false,
    };
    let claimed = apply(
        &start,
        &PlansChange::Claim {
            claim: held,
            revision: REVISION,
        },
    );
    let ran = apply(
        &claimed,
        &PlansChange::Settle {
            claim: claimed.claim.unwrap(),
        },
    );
    assert!(!ran.table[0].unwrap().enabled);

    let back_ms = due_ms - 5 * MINUTE_MS;
    let undo = change(&ran, 0, seconds(due_ms), MealSkip::Unskip, back_ms, europe);
    assert_eq!(undo, Err(Refusal::Invalid));
}
