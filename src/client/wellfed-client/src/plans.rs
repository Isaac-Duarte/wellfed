//! Changing the feeding plans, and what they come to today. The feeder
//! replaces the whole table against a generation, so a change is a read, an
//! edit of that table ([`edited`]) and a write; every client makes the same
//! edit here, and a stale write is refused rather than merged.
//!
//! Skipping one of today's meals is no edit of the table: it names the
//! meal's instant, which [`to_skip`] and [`to_undo`] find in the plans as
//! read, so no user types a time. The feeder checks the same rules again
//! against its own clock and refuses what no longer holds.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use wellfed_model::{CALL_PLAYS_MAX, PLANS, PORTIONS_MAX, Plan, Plans, TimeOfDay, Weekdays};

use crate::history::Feed;
use crate::time::{core_plan, next_meal, offsets};

/// What a change does to the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Into the first free slot.
    Add(Plan),
    /// What it does not name stays.
    Change {
        slot: u8,
        at: Option<TimeOfDay>,
        portions: Option<u8>,
        days: Option<Weekdays>,
        call: Option<u8>,
        enabled: Option<bool>,
    },
    Enabled {
        slot: u8,
        enabled: bool,
    },
    Delete {
        slot: u8,
    },
}

/// Why an edit cannot be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error("slot {slot} holds no plan")]
    Empty { slot: u8 },
    #[error("every one of the {PLANS} slots holds a plan")]
    Full,
    #[error("the change names nothing to change")]
    Nothing,
    #[error(
        "a plan feeds at a minute of the day, 1 to {PORTIONS_MAX} portions, with up to {CALL_PLAYS_MAX} calls"
    )]
    Invalid,
}

/// The table `plans` holds with `edit` applied.
pub fn edited(plans: &Plans, edit: Edit) -> Result<[Option<Plan>; PLANS], EditError> {
    let mut table = plans.plans;
    let filled = |table: &[Option<Plan>; PLANS], slot: u8| {
        table
            .get(usize::from(slot))
            .copied()
            .flatten()
            .ok_or(EditError::Empty { slot })
    };
    let checked = |plan: Plan| {
        if plan.valid() {
            Ok(plan)
        } else {
            Err(EditError::Invalid)
        }
    };

    match edit {
        Edit::Add(plan) => {
            let free = table
                .iter_mut()
                .find(|slot| slot.is_none())
                .ok_or(EditError::Full)?;
            *free = Some(checked(plan)?);
        }
        Edit::Change {
            slot,
            at,
            portions,
            days,
            call,
            enabled,
        } => {
            let mut plan = filled(&table, slot)?;
            let named = at.is_some()
                || portions.is_some()
                || days.is_some()
                || call.is_some()
                || enabled.is_some();
            if !named {
                return Err(EditError::Nothing);
            }
            plan.minute = at.map_or(plan.minute, |at| at.0);
            plan.portions = portions.unwrap_or(plan.portions);
            plan.days = days.unwrap_or(plan.days);
            plan.call = call.unwrap_or(plan.call);
            plan.enabled = enabled.unwrap_or(plan.enabled);
            table[usize::from(slot)] = Some(checked(plan)?);
        }
        Edit::Enabled { slot, enabled } => {
            let plan = filled(&table, slot)?;
            table[usize::from(slot)] = Some(Plan { enabled, ..plan });
        }
        Edit::Delete { slot } => {
            filled(&table, slot)?;
            table[usize::from(slot)] = None;
        }
    }
    Ok(table)
}

/// Why a meal cannot be skipped, or its skip undone. The feeder refuses the
/// same, by its own clock, if the plans moved on since they were read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SkipError {
    #[error("slot {slot} holds no plan")]
    Empty { slot: u8 },
    #[error("plan {slot} is disabled")]
    Disabled { slot: u8 },
    #[error("plan {slot} has no meal left today")]
    NoneToday { slot: u8 },
    #[error("plan {slot}'s next meal is already skipped")]
    Skipped { slot: u8 },
    #[error("plan {slot} has no skipped meal")]
    NotSkipped { slot: u8 },
    #[error("plan {slot}'s skipped meal is already due, so its skip stays")]
    Due { slot: u8 },
}

/// The meal of slot `slot` a skip names: the plan's next, while it falls on
/// today's date in `zone` and is not skipped already.
pub fn to_skip(
    plans: &Plans,
    slot: u8,
    now: Timestamp,
    zone: &TimeZone,
) -> Result<Timestamp, SkipError> {
    let index = usize::from(slot);
    let plan = plans
        .plans
        .get(index)
        .copied()
        .flatten()
        .ok_or(SkipError::Empty { slot })?;
    if !plan.enabled {
        return Err(SkipError::Disabled { slot });
    }
    if plans.skipped.get(index).is_some_and(|at| at > now) {
        return Err(SkipError::Skipped { slot });
    }

    let now_ms = u64::try_from(now.as_millisecond()).map_err(|_| SkipError::NoneToday { slot })?;
    let due = wellfed_core::schedule::next_due(&core_plan(&plan), now_ms, offsets(zone))
        .and_then(|ms| i64::try_from(ms).ok())
        .and_then(|ms| Timestamp::from_millisecond(ms).ok())
        .filter(|at| same_day(*at, zone, now.to_zoned(zone.clone()).date()))
        .ok_or(SkipError::NoneToday { slot })?;
    Ok(due)
}

/// The meal of slot `slot` an undo names: the skipped one, while it is
/// still to come. At its instant it is the schedule's, so the skip stays.
pub fn to_undo(plans: &Plans, slot: u8, now: Timestamp) -> Result<Timestamp, SkipError> {
    let index = usize::from(slot);
    if plans.plans.get(index).copied().flatten().is_none() {
        return Err(SkipError::Empty { slot });
    }
    let at = plans
        .skipped
        .get(index)
        .ok_or(SkipError::NotSkipped { slot })?;
    if at <= now {
        return Err(SkipError::Due { slot });
    }
    Ok(at)
}

/// How one of today's meals stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MealState {
    /// Its plan fed today: the feed's result.
    Fed(Feed),
    /// Its time has passed with no feed recorded for it: late, missed, or
    /// the plan changed after the feed that ran.
    Passed,
    /// The next meal of all the plans.
    Next {
        at: Timestamp,
    },
    Later,
    /// Skipped, and still to come: an undo before `at` brings it back.
    Skipped {
        at: Timestamp,
    },
    /// Skipped, and its time has come: it stays skipped.
    SkipKept,
}

/// What a client may offer for one of today's meals, with the instant the
/// request names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MealAction {
    /// The plan's next meal, today and still to come ([`to_skip`]).
    Skip { due: Timestamp },
    /// The skipped meal, still to come ([`to_undo`]).
    Undo { due: Timestamp },
}

/// One of today's meals by the feeder's local calendar: an enabled plan
/// that runs today, or one that fed or was skipped today.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Meal {
    pub slot: u8,
    pub minute: u16,
    pub portions: u8,
    pub call: u8,
    pub state: MealState,
    /// `None` for a meal that fed, passed, or whose skip stays.
    pub action: Option<MealAction>,
}

/// Today's meals in `zone`, earliest first. `feeds` is the history as far as
/// it was read: each meal takes the last feed its plan's slot fed today, at
/// or after its minute, so a plan moved later today shows nothing as fed.
pub fn today(plans: &Plans, feeds: &[Feed], now: Timestamp, zone: &TimeZone) -> Vec<Meal> {
    let local = now.to_zoned(zone.clone());
    let day = local.date();
    let minute_now =
        u16::try_from(i32::from(local.hour()) * 60 + i32::from(local.minute())).unwrap_or(u16::MAX);
    let next = next_meal(plans, now, zone);

    let mut meals: Vec<Meal> = plans
        .plans
        .iter()
        .enumerate()
        .filter_map(|(slot, plan)| {
            let plan = (*plan)?;
            let slot = u8::try_from(slot).ok()?;
            let runs = plan.days.bits() == 0 || runs_on(plan.days, day);
            // A once plan that ran, or was skipped, stays in its slot
            // disabled: it still shows as today's meal when its feed or
            // its skip is today's.
            let fed = last_feed(feeds, slot, plan.minute, zone, day);
            let skipped = plans
                .skipped
                .get(usize::from(slot))
                .filter(|at| same_day(*at, zone, day));
            if !(plan.enabled && runs) && fed.is_none() && skipped.is_none() {
                return None;
            }

            let state = match (fed, skipped, next) {
                (Some(feed), _, _) => MealState::Fed(feed),
                (None, Some(at), _) if at > now => MealState::Skipped { at },
                (None, Some(_), _) => MealState::SkipKept,
                (None, None, Some(next)) if next.slot == slot && same_day(next.at, zone, day) => {
                    MealState::Next { at: next.at }
                }
                (None, None, _) if plan.minute <= minute_now => MealState::Passed,
                (None, None, _) => MealState::Later,
            };
            let action = match state {
                MealState::Skipped { at } => Some(MealAction::Undo { due: at }),
                MealState::Next { .. } | MealState::Later => to_skip(plans, slot, now, zone)
                    .ok()
                    .map(|due| MealAction::Skip { due }),
                MealState::Fed(_) | MealState::Passed | MealState::SkipKept => None,
            };
            Some(Meal {
                slot,
                minute: plan.minute,
                portions: plan.portions,
                call: plan.call,
                state,
                action,
            })
        })
        .collect();
    meals.sort_by_key(|meal| (meal.minute, meal.slot));
    meals
}

fn runs_on(days: Weekdays, day: jiff::civil::Date) -> bool {
    let bit = u32::try_from(day.weekday().to_monday_zero_offset()).unwrap_or(0);
    u32::from(days.bits()) & (1 << bit) != 0
}

fn same_day(at: Timestamp, zone: &TimeZone, day: jiff::civil::Date) -> bool {
    at.to_zoned(zone.clone()).date() == day
}

fn last_feed(
    feeds: &[Feed],
    slot: u8,
    minute: u16,
    zone: &TimeZone,
    day: jiff::civil::Date,
) -> Option<Feed> {
    feeds.iter().rev().copied().find(|feed| {
        let ours =
            matches!(feed.by, Some(wellfed_model::Source::Schedule { plan }) if plan == slot);
        ours && feed.at.is_some_and(|at| {
            let local = at.to_zoned(zone.clone());
            let at_minute = i32::from(local.hour()) * 60 + i32::from(local.minute());
            local.date() == day && at_minute >= i32::from(minute)
        })
    })
}

#[cfg(test)]
mod tests {
    use wellfed_model::{FeedOutcome, FeedReason, FoodLevel, Generation, Source};

    use super::*;
    use crate::history::Delivered;

    fn plans(table: [Option<Plan>; PLANS]) -> Plans {
        Plans {
            generation: Generation(3),
            writable: true,
            plans: table,
            skipped: wellfed_model::SkippedMeals::NONE,
        }
    }

    const BREAKFAST: Plan = Plan {
        minute: 7 * 60 + 30,
        portions: 2,
        days: Weekdays::DAILY,
        enabled: true,
        call: 0,
    };

    #[test]
    fn a_new_plan_takes_the_first_free_slot() {
        let mut table = [None; PLANS];
        table[0] = Some(BREAKFAST);
        let table = edited(&plans(table), Edit::Add(BREAKFAST)).unwrap();
        assert_eq!(table[1], Some(BREAKFAST));

        let full = plans([Some(BREAKFAST); PLANS]);
        assert_eq!(edited(&full, Edit::Add(BREAKFAST)), Err(EditError::Full));
    }

    #[test]
    fn an_edit_keeps_what_it_does_not_name() {
        let mut table = [None; PLANS];
        table[4] = Some(BREAKFAST);
        let change = Edit::Change {
            slot: 4,
            at: None,
            portions: Some(3),
            days: None,
            call: Some(2),
            enabled: None,
        };
        let table = edited(&plans(table), change).unwrap();
        assert_eq!(
            table[4],
            Some(Plan {
                portions: 3,
                call: 2,
                ..BREAKFAST
            })
        );
    }

    #[test]
    fn an_empty_slot_cannot_be_changed() {
        let table = plans([None; PLANS]);
        for edit in [
            Edit::Delete { slot: 2 },
            Edit::Enabled {
                slot: 2,
                enabled: true,
            },
        ] {
            assert_eq!(edited(&table, edit), Err(EditError::Empty { slot: 2 }));
        }
    }

    #[test]
    fn a_change_that_names_nothing_or_breaks_a_limit_is_refused() {
        let mut table = [None; PLANS];
        table[1] = Some(BREAKFAST);
        let change = |portions, call| Edit::Change {
            slot: 1,
            at: None,
            portions,
            days: None,
            call,
            enabled: None,
        };
        assert_eq!(
            edited(&plans(table), change(None, None)),
            Err(EditError::Nothing)
        );
        assert_eq!(
            edited(&plans(table), change(Some(49), None)),
            Err(EditError::Invalid)
        );
        assert_eq!(
            edited(&plans(table), change(None, Some(6))),
            Err(EditError::Invalid)
        );
    }

    fn fed(slot: u8, at: Timestamp) -> Feed {
        Feed {
            epoch: 1,
            seq: 9,
            feed: 4,
            at: Some(at),
            by: Some(Source::Schedule { plan: slot }),
            requested: 2,
            delivered: Delivered::Counted(2),
            outcome: FeedOutcome::Done,
            reason: FeedReason::Done,
            hopper: FoodLevel::Present,
        }
    }

    #[test]
    fn today_says_how_each_meal_stands() {
        let zone = TimeZone::UTC;
        // A Friday, 12:04.
        let now: Timestamp = "2026-10-30T12:04:00Z".parse().unwrap();
        let mut table = [None; PLANS];
        table[0] = Some(BREAKFAST);
        table[1] = Some(Plan {
            minute: 12 * 60 + 30,
            ..BREAKFAST
        });
        table[2] = Some(Plan {
            minute: 18 * 60 + 30,
            portions: 3,
            call: 1,
            ..BREAKFAST
        });
        // Mondays only, so not today's.
        table[3] = Some(Plan {
            minute: 9 * 60,
            days: Weekdays::from_bits(1),
            ..BREAKFAST
        });
        table[4] = Some(Plan {
            minute: 8 * 60,
            ..BREAKFAST
        });
        table[5] = Some(Plan {
            minute: 6 * 60,
            enabled: false,
            ..BREAKFAST
        });

        let breakfast = fed(0, "2026-10-30T07:30:40Z".parse().unwrap());
        // Yesterday's feed of slot 4 is not today's.
        let yesterday = fed(4, "2026-10-29T08:00:40Z".parse().unwrap());
        let meals = today(&plans(table), &[yesterday, breakfast], now, &zone);

        let states: Vec<_> = meals.iter().map(|meal| (meal.slot, meal.state)).collect();
        assert_eq!(
            states,
            [
                (0, MealState::Fed(breakfast)),
                (4, MealState::Passed),
                (
                    1,
                    MealState::Next {
                        at: "2026-10-30T12:30:00Z".parse().unwrap()
                    }
                ),
                (2, MealState::Later),
            ]
        );
        assert_eq!(
            meals[3].action,
            Some(MealAction::Skip {
                due: "2026-10-30T18:30:00Z".parse().unwrap()
            }),
            "a later meal today can be skipped too"
        );
        assert_eq!((meals[0].action, meals[1].action), (None, None));
    }

    #[test]
    fn a_plan_that_ran_once_still_shows_todays_feed() {
        let zone = TimeZone::UTC;
        let now: Timestamp = "2026-10-30T12:04:00Z".parse().unwrap();
        let mut table = [None; PLANS];
        table[0] = Some(Plan {
            days: Weekdays::ONCE,
            enabled: false,
            ..BREAKFAST
        });
        let once = fed(0, "2026-10-30T07:30:40Z".parse().unwrap());

        let meals = today(&plans(table), &[once], now, &zone);
        assert_eq!(meals.len(), 1);
        assert_eq!(meals[0].state, MealState::Fed(once));
    }

    /// Plans with breakfast (slot 0, 07:30), lunch (slot 1, 12:30) and
    /// dinner (slot 2, 18:30), daily, and `skipped` per slot.
    fn meals_skipping(skipped: &[(usize, &str)]) -> Plans {
        let mut table = [None; PLANS];
        table[0] = Some(BREAKFAST);
        table[1] = Some(Plan {
            minute: 12 * 60 + 30,
            ..BREAKFAST
        });
        table[2] = Some(Plan {
            minute: 18 * 60 + 30,
            ..BREAKFAST
        });
        let mut seconds = [None; PLANS];
        for &(slot, at) in skipped {
            let at: Timestamp = at.parse().unwrap();
            seconds[slot] = Some(u32::try_from(at.as_second()).unwrap());
        }
        Plans {
            skipped: wellfed_model::SkippedMeals::from_seconds(seconds),
            ..plans(table)
        }
    }

    /// A skipped meal still to come can be undone; once its time came the
    /// skip stays. The next meal passes over it.
    #[test]
    fn today_shows_a_skip_until_its_time_and_after() {
        let zone = TimeZone::UTC;
        let now: Timestamp = "2026-10-30T12:04:00Z".parse().unwrap();
        let skipped = meals_skipping(&[(0, "2026-10-30T07:30:00Z"), (1, "2026-10-30T12:30:00Z")]);

        let meals = today(&skipped, &[], now, &zone);
        let states: Vec<_> = meals.iter().map(|meal| (meal.slot, meal.state)).collect();
        assert_eq!(
            states,
            [
                (0, MealState::SkipKept),
                (
                    1,
                    MealState::Skipped {
                        at: "2026-10-30T12:30:00Z".parse().unwrap()
                    }
                ),
                (
                    2,
                    MealState::Next {
                        at: "2026-10-30T18:30:00Z".parse().unwrap()
                    }
                ),
            ]
        );
        let actions: Vec<_> = meals.iter().map(|meal| meal.action).collect();
        assert_eq!(
            actions,
            [
                None,
                Some(MealAction::Undo {
                    due: "2026-10-30T12:30:00Z".parse().unwrap()
                }),
                Some(MealAction::Skip {
                    due: "2026-10-30T18:30:00Z".parse().unwrap()
                }),
            ],
            "a kept skip offers nothing"
        );
    }

    /// Yesterday's skip says nothing of today's meal.
    #[test]
    fn a_skip_of_another_day_is_not_todays() {
        let zone = TimeZone::UTC;
        let now: Timestamp = "2026-10-30T06:00:00Z".parse().unwrap();
        let plans = meals_skipping(&[(0, "2026-10-29T07:30:00Z")]);

        let meals = today(&plans, &[], now, &zone);
        assert_eq!(
            meals[0].state,
            MealState::Next {
                at: "2026-10-30T07:30:00Z".parse().unwrap()
            }
        );
    }

    /// A plan that runs once is disabled by its skip, and still shows as
    /// today's skipped meal.
    #[test]
    fn a_once_plan_its_skip_disabled_shows_as_skipped() {
        let zone = TimeZone::UTC;
        let now: Timestamp = "2026-10-30T06:00:00Z".parse().unwrap();
        let mut table = [None; PLANS];
        table[0] = Some(Plan {
            days: Weekdays::ONCE,
            enabled: false,
            ..BREAKFAST
        });
        let mut seconds = [None; PLANS];
        let at: Timestamp = "2026-10-30T07:30:00Z".parse().unwrap();
        seconds[0] = Some(u32::try_from(at.as_second()).unwrap());
        let plans = Plans {
            skipped: wellfed_model::SkippedMeals::from_seconds(seconds),
            ..plans(table)
        };

        let meals = today(&plans, &[], now, &zone);
        assert_eq!(meals.len(), 1);
        assert_eq!(meals[0].state, MealState::Skipped { at });
        assert_eq!(to_undo(&plans, 0, now), Ok(at));
    }

    #[test]
    fn a_skip_names_the_plans_next_meal_today() {
        let zone = TimeZone::UTC;
        let now: Timestamp = "2026-10-30T12:04:00Z".parse().unwrap();
        let none = meals_skipping(&[]);

        assert_eq!(
            to_skip(&none, 1, now, &zone),
            Ok("2026-10-30T12:30:00Z".parse().unwrap())
        );
        assert_eq!(
            to_skip(&none, 0, now, &zone),
            Err(SkipError::NoneToday { slot: 0 }),
            "breakfast is tomorrow's"
        );
        assert_eq!(
            to_skip(&none, 5, now, &zone),
            Err(SkipError::Empty { slot: 5 })
        );

        let lunch = meals_skipping(&[(1, "2026-10-30T12:30:00Z")]);
        assert_eq!(
            to_skip(&lunch, 1, now, &zone),
            Err(SkipError::Skipped { slot: 1 })
        );

        let mut disabled = none;
        disabled.plans[2] = disabled.plans[2].map(|plan| Plan {
            enabled: false,
            ..plan
        });
        assert_eq!(
            to_skip(&disabled, 2, now, &zone),
            Err(SkipError::Disabled { slot: 2 })
        );
    }

    #[test]
    fn an_undo_names_the_skipped_meal_until_its_time() {
        let lunch: Timestamp = "2026-10-30T12:30:00Z".parse().unwrap();
        let plans = meals_skipping(&[(1, "2026-10-30T12:30:00Z")]);
        let before: Timestamp = "2026-10-30T12:04:00Z".parse().unwrap();

        assert_eq!(to_undo(&plans, 1, before), Ok(lunch));
        assert_eq!(to_undo(&plans, 1, lunch), Err(SkipError::Due { slot: 1 }));
        assert_eq!(
            to_undo(&plans, 2, before),
            Err(SkipError::NotSkipped { slot: 2 })
        );
        assert_eq!(
            to_undo(&plans, 7, before),
            Err(SkipError::Empty { slot: 7 })
        );
    }
}
