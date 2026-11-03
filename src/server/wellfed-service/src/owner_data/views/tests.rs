//! The views' arithmetic under generated feeds, refills, readings and
//! links: a refill resets the count and the beam never does, a shared
//! feeder's intake splits evenly with nothing made or lost, every feed
//! lands on one household day, and anomalies are exactly the feeds that
//! went wrong.

use jiff::civil::Date;
use jiff::tz::TimeZone;
use proptest::prelude::*;
use proptest::sample::select;
use uuid::Uuid;
use wellfed_api::{FoodId, FoodStats, PetId, PetLink, RefillId, UserId};

use super::*;

const KITCHEN: FeederId = FeederId(Uuid::from_u128(1));
/// 2026-11-01T00:00:00Z: generated times are seconds after it.
const BASE: i64 = 1_793_491_200;

fn at(second: i64) -> Timestamp {
    Timestamp::from_second(BASE + second).unwrap()
}

fn pet(n: u128) -> PetId {
    PetId(Uuid::from_u128(100 + n))
}

fn food(grams_per_cup: u32, kcal_per_kg: u32) -> Food {
    Food {
        id: FoodId(Uuid::from_u128(7)),
        brand: "Acme".into(),
        name: "Dry".into(),
        kcal_per_kg,
        bag_grams: 2000,
        grams_per_cup,
    }
}

fn refill(second: i64, grams: u32) -> Refill {
    Refill {
        id: RefillId(Uuid::from_u128(9)),
        feeder: KITCHEN,
        at: at(second),
        grams,
        full_bag: false,
        food: None,
        user: UserId(Uuid::from_u128(3)),
    }
}

fn done(second: i64, portions: u8, food: Option<Food>) -> FeedRow {
    FeedRow {
        feeder: KITCHEN,
        feed: 1,
        at: at(second),
        requested: portions,
        completed: portions,
        outcome: FeedOutcome::Done,
        reason: FeedReason::Done,
        food,
    }
}

fn weight() -> impl Strategy<Value = PortionWeight> {
    prop_oneof![
        (1_000..=50_000u32).prop_map(|portion_mg| PortionWeight::Calibration { portion_mg }),
        Just(PortionWeight::TwelfthCup),
    ]
}

/// A feed within the first 30 days, of any outcome.
fn feed() -> impl Strategy<Value = FeedRow> {
    let outcome = select(vec![
        FeedOutcome::Done,
        FeedOutcome::Cancelled,
        FeedOutcome::Expired,
        FeedOutcome::Failed,
        FeedOutcome::Uncertain,
    ]);
    let food = proptest::option::of((1..=1000u32, 1..=10_000u32).prop_map(|(c, k)| food(c, k)));
    (0..30 * 86_400i64, 1..=48u8, any::<u8>(), outcome, food).prop_map(
        |(second, requested, completed, outcome, food)| FeedRow {
            feeder: KITCHEN,
            feed: u32::try_from(second).unwrap(),
            at: at(second),
            requested,
            completed: completed % (requested + 1),
            outcome,
            reason: FeedReason::Done,
            food,
        },
    )
}

proptest! {
    #[test]
    fn feeds_before_the_refill_do_not_count(
        feeds in proptest::collection::vec(feed(), 0..40),
        weight in weight(),
        grams in 1..=100_000u32,
        refill_at in 0..30 * 86_400i64,
    ) {
        let refill = refill(refill_at, grams);
        let after: Vec<FeedRow> = feeds.iter().filter(|f| f.at >= refill.at).cloned().collect();
        let now = at(31 * 86_400);

        let all = food_left(now, &refill, &feeds, weight, None);

        prop_assert_eq!(all, food_left(now, &refill, &after, weight, None));
        prop_assert!(all.grams_left_least <= all.grams_left);
        prop_assert!(all.grams_left <= grams);
        if all.grams_used <= grams {
            prop_assert!(all.grams_left.abs_diff(grams - all.grams_used) <= 1);
        }
    }

    #[test]
    fn another_feed_never_adds_food(
        feeds in proptest::collection::vec(feed(), 0..40),
        extra in feed(),
        weight in weight(),
        grams in 1..=100_000u32,
    ) {
        let refill = refill(0, grams);
        let now = at(31 * 86_400);
        let before = food_left(now, &refill, &feeds, weight, None);
        let mut more = feeds.clone();
        more.push(extra);

        let after = food_left(now, &refill, &more, weight, None);

        prop_assert!(after.grams_left <= before.grams_left);
        prop_assert!(after.portions >= before.portions);
    }

    /// Whatever the beam saw, only a recorded refill resets the count: the
    /// food view's count is the same with or without the readings.
    #[test]
    fn the_beam_flags_a_refill_and_never_resets_the_count(
        present in proptest::collection::vec(proptest::option::of(any::<bool>()), 0..20),
    ) {
        let readings: Vec<FoodReading> = present.iter().enumerate().map(|(n, &present)| FoodReading {
            received: at(i64::try_from(n).unwrap() * 60),
            boot_count: Some(1),
            food: FoodStats { present, ran_low: 0, refilled: 0, feeds_since_refill: None },
        }).collect();
        let known: Vec<bool> = present.iter().flatten().copied().collect();
        let refilled = known.windows(2).any(|pair| pair == [false, true]);

        let seen = refilled_unrecorded(&readings);

        prop_assert_eq!(seen.is_some(), refilled);
        if let Some(seen) = seen {
            let reading = readings.iter().find(|r| r.received == seen).unwrap();
            prop_assert_eq!(reading.food.present, Some(true));
        }
    }

    #[test]
    fn a_shared_feeder_splits_evenly_and_loses_nothing(
        feeds in proptest::collection::vec(feed(), 1..30),
        weight in weight(),
        pets in 1..=4u128,
    ) {
        let links: Vec<PetLink> = (0..pets).map(|n| PetLink {
            pet: pet(n), feeder: KITCHEN, from: at(0), to: None,
        }).collect();
        let served = [Served { feeder: KITCHEN, weight, links: &links, feeds: &feeds }];
        let dates = days(date(2026, 11, 30), 30);
        let total = Tally::of(&feeds, weight).mg;

        let shares: Vec<_> = (0..pets)
            .map(|n| intake(pet(n), None, &dates, &TimeZone::UTC, &served))
            .collect();

        let (feeders, first) = &shares[0];
        prop_assert_eq!(feeders[0].shared, pets > 1);
        for (_, days) in &shares {
            prop_assert_eq!(days, first, "every pet gets the same share");
        }
        let one: u64 = first.iter().map(|day| u64::from(day.grams)).sum();
        let all = one * u64::try_from(pets).unwrap() * MG_PER_G;
        let slack = u64::try_from(feeds.len() + first.len()).unwrap() * 1_000 * 4;
        prop_assert!(all.abs_diff(total) <= slack, "{all} mg shared of {total} mg");
        for day in first {
            prop_assert!(!day.estimate || pets > 1, "one pet's intake is not an estimate");
            prop_assert!(day.estimate || pets == 1 || day.grams + day.unweighed_portions == 0);
        }
    }

    #[test]
    fn every_feed_lands_on_one_household_day(
        feeds in proptest::collection::vec(feed(), 0..30),
        weight in weight(),
        zone in select(vec!["UTC", "America/Denver", "Pacific/Kiritimati", "Asia/Kolkata"]),
    ) {
        let zone = wellfed_mirror::Zone::stored(zone, at(0)).unwrap().tz().clone();
        let links = [PetLink { pet: pet(0), feeder: KITCHEN, from: at(-86_400), to: None }];
        let served = [Served { feeder: KITCHEN, weight, links: &links, feeds: &feeds }];
        let dates = days(date(2026, 12, 2), 33);

        let (_, days) = intake(pet(0), None, &dates, &zone, &served);

        prop_assert_eq!(days.len(), 33);
        let mg: u64 = feeds.iter().map(|feed| Tally::of([feed], weight).mg).sum();
        let grams: u64 = days.iter().map(|day| u64::from(day.grams)).sum();
        prop_assert!((grams * MG_PER_G).abs_diff(mg) <= 33 * 500);
    }

    #[test]
    fn anomalies_are_the_feeds_that_went_wrong_in_time_order(
        feeds in proptest::collection::vec(feed(), 0..40),
    ) {
        let found = anomalies(&feeds, &[]);

        let wrong = feeds.iter().filter(|feed| match feed.outcome {
            FeedOutcome::Uncertain => true,
            FeedOutcome::Cancelled => false,
            _ => feed.completed < feed.requested,
        }).count();
        prop_assert_eq!(found.len(), wrong);
        let times: Vec<Timestamp> = found.iter().map(|anomaly| match anomaly {
            FeedAnomaly::Uncertain { at, .. } | FeedAnomaly::Short { at, .. } => *at,
            FeedAnomaly::Missed { due, .. } => *due,
        }).collect();
        prop_assert!(times.is_sorted());
    }
}

fn date(year: i16, month: i8, day: i8) -> Date {
    jiff::civil::date(year, month, day)
}

#[test]
fn a_feed_drops_at_the_feeders_time_when_its_clock_was_valid() {
    let value = Feed {
        feed: 4,
        by: wellfed_api::Source::Feeder,
        requested: 2,
        completed: 2,
        outcome: FeedOutcome::Done,
        reason: FeedReason::Done,
    };
    let feed = |at| Projected {
        epoch: 1,
        seq: 9,
        at,
        received: self::at(500),
        value,
    };

    assert_eq!(dropped(&feed(Some(at(100)))), at(100));
    assert_eq!(dropped(&feed(None)), at(500));
}

#[test]
fn a_portion_is_the_calibration_or_a_twelfth_cup() {
    let dry = food(120, 3600);

    assert_eq!(
        portion_mg(portion_weight(Some(9_200)), Some(&dry)),
        Some(9_200)
    );
    assert_eq!(portion_mg(portion_weight(None), Some(&dry)), Some(10_000));
    assert_eq!(portion_mg(portion_weight(None), None), None);
}

#[test]
fn twice_a_day_for_a_week_is_the_meal_rate() {
    let feeds: Vec<FeedRow> = (0..14)
        .map(|meal| done(meal * 12 * 3600, 2, Some(food(120, 3600))))
        .collect();
    let now = at(13 * 12 * 3600 + 3600);

    let rate = grams_per_day(now, 7, &feeds, PortionWeight::TwelfthCup);

    // Two meals of two 10 g portions a day.
    assert!(rate.is_some_and(|rate| rate.abs_diff(40) <= 3), "{rate:?}");
    assert_eq!(
        grams_per_day(at(3600), 7, &feeds[..1], PortionWeight::TwelfthCup),
        None
    );
}

#[test]
fn the_hopper_runs_out_at_the_rate() {
    let feeds = [done(3600, 3, None)];
    let weight = PortionWeight::Calibration { portion_mg: 10_000 };

    let left = food_left(at(7200), &refill(0, 1_030), &feeds, weight, Some(100));

    assert_eq!((left.grams_used, left.grams_left), (30, 1_000));
    assert_eq!(left.empty_at, Some(at(7200 + 10 * 86_400)));
}

#[test]
fn a_counted_refill_in_one_boot_is_seen_without_a_low_reading() {
    let reading = |second, boot, refilled| FoodReading {
        received: at(second),
        boot_count: Some(boot),
        food: FoodStats {
            present: Some(true),
            ran_low: 0,
            refilled,
            feeds_since_refill: None,
        },
    };

    let rebooted = [reading(0, 1, 3), reading(60, 2, 4)];
    let counted = [reading(0, 1, 3), reading(60, 1, 4)];

    assert_eq!(refilled_unrecorded(&rebooted), None);
    assert_eq!(refilled_unrecorded(&counted), Some(at(60)));
}

#[test]
fn a_feed_on_a_feeder_the_pet_left_is_not_its_own() {
    let links = [
        PetLink {
            pet: pet(0),
            feeder: KITCHEN,
            from: at(0),
            to: Some(at(1000)),
        },
        PetLink {
            pet: pet(1),
            feeder: KITCHEN,
            from: at(0),
            to: None,
        },
    ];
    let feeds = [
        done(500, 2, Some(food(120, 3600))),
        done(2000, 2, Some(food(120, 3600))),
    ];
    let served = [Served {
        feeder: KITCHEN,
        weight: PortionWeight::TwelfthCup,
        links: &links,
        feeds: &feeds,
    }];
    let dates = days(date(2026, 11, 1), 1);

    let (_, mine) = intake(pet(0), Some(36), &dates, &TimeZone::UTC, &served);
    let (_, theirs) = intake(pet(1), None, &dates, &TimeZone::UTC, &served);

    // 20 g each feed; the first is split, the second is pet 1's alone.
    assert_eq!(
        (mine[0].grams, mine[0].kcal, mine[0].target_percent),
        (10, 36, Some(100))
    );
    assert!(mine[0].estimate);
    assert_eq!(theirs[0].grams, 30);
}
