//! The views' arithmetic under generated feeds, refills and readings: a
//! refill resets the count and the beam never does, an uncalibrated
//! feeder is never weighed, and anomalies are exactly the feeds that went
//! wrong. [`intake`] holds a pet's intake.

mod intake;

use jiff::civil::Date;
use proptest::prelude::*;
use proptest::sample::select;
use uuid::Uuid;
use wellfed_api::{FoodId, FoodStats, PetId, RefillId, UserId};

use super::*;

const KITCHEN: FeederId = FeederId(Uuid::from_u128(1));
/// 2026-11-01T00:00:00Z: generated times are seconds after it.
const BASE: i64 = 1_793_491_200;
const TEN_GRAMS: PortionWeight = PortionWeight::Calibration { portion_mg: 10_000 };

fn at(second: i64) -> Timestamp {
    Timestamp::from_second(BASE + second).unwrap()
}

fn pet(n: u128) -> PetId {
    PetId(Uuid::from_u128(100 + n))
}

fn food(kcal_per_kg: u32) -> Food {
    Food {
        id: FoodId(Uuid::from_u128(7)),
        brand: "Acme".into(),
        name: "Dry".into(),
        kcal_per_kg,
        bag_grams: 2000,
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

fn calibration() -> impl Strategy<Value = PortionWeight> {
    (1_000..=50_000u32).prop_map(|portion_mg| PortionWeight::Calibration { portion_mg })
}

fn weight() -> impl Strategy<Value = PortionWeight> {
    prop_oneof![calibration(), Just(PortionWeight::Uncalibrated)]
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
    let food = proptest::option::of((1..=10_000u32).prop_map(food));
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

fn feeds() -> impl Strategy<Value = Vec<FeedRow>> {
    proptest::collection::vec(feed(), 0..40)
}

proptest! {
    #[test]
    fn feeds_before_the_refill_do_not_count(
        feeds in feeds(),
        weight in weight(),
        grams in 1..=100_000u32,
        refill_at in 0..30 * 86_400i64,
    ) {
        let refill = refill(refill_at, grams);
        let after: Vec<FeedRow> = feeds.iter().filter(|f| f.at >= refill.at).cloned().collect();
        let now = at(31 * 86_400);

        let all = since_refill(now, &refill, &feeds, weight, None);

        prop_assert_eq!(all, since_refill(now, &refill, &after, weight, None));
        prop_assert_eq!(all.left.is_some(), weight != PortionWeight::Uncalibrated);
        if let Some(left) = all.left {
            prop_assert!(left.grams_left_least <= left.grams_left);
            prop_assert!(left.grams_left <= grams);
            if left.grams_used <= grams {
                prop_assert!(left.grams_left.abs_diff(grams - left.grams_used) <= 1);
            }
        }
    }

    #[test]
    fn another_feed_never_adds_food(
        feeds in feeds(),
        extra in feed(),
        weight in calibration(),
        grams in 1..=100_000u32,
    ) {
        let refill = refill(0, grams);
        let now = at(31 * 86_400);
        let before = since_refill(now, &refill, &feeds, weight, None);
        let mut more = feeds.clone();
        more.push(extra);

        let after = since_refill(now, &refill, &more, weight, None);

        prop_assert!(after.left.unwrap().grams_left <= before.left.unwrap().grams_left);
        prop_assert!(after.portions >= before.portions);
    }

    /// An uncalibrated feeder's rate is portions only, and a calibration
    /// weighs the same portions.
    #[test]
    fn only_a_calibration_weighs_the_rate(feeds in feeds(), weight in calibration()) {
        let now = at(31 * 86_400);

        let counted = rate(now, 7, &feeds, PortionWeight::Uncalibrated);
        let weighed = rate(now, 7, &feeds, weight);

        prop_assert_eq!(counted.and_then(|rate| rate.grams_per_day), None);
        prop_assert_eq!(counted.map(|rate| rate.portions), weighed.map(|rate| rate.portions));
        prop_assert_eq!(weighed.is_some(), weighed.is_some_and(|rate| rate.grams_per_day.is_some()));
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
    fn anomalies_are_the_feeds_that_went_wrong_in_time_order(feeds in feeds()) {
        let found = anomalies(&feeds, &[]);

        let wrong = feeds.iter().filter(|feed| match feed.outcome {
            FeedOutcome::Uncertain => true,
            FeedOutcome::Cancelled => false,
            _ => feed.completed < feed.requested,
        }).count();
        prop_assert_eq!(found.len(), wrong);
        let times: Vec<Timestamp> = found.iter().filter_map(|anomaly| match anomaly {
            FeedAnomaly::Uncertain { at, .. } | FeedAnomaly::Short { at, .. } => Some(*at),
            FeedAnomaly::Missed { due, .. } => Some(*due),
            FeedAnomaly::Unknown => None,
        }).collect();
        prop_assert_eq!(times.len(), found.len(), "this server names every anomaly it finds");
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
fn only_the_calibration_weighs_a_portion() {
    assert_eq!(portion_mg(portion_weight(Some(9_200))), Some(9_200));
    assert_eq!(portion_weight(None), PortionWeight::Uncalibrated);
    assert_eq!(portion_mg(PortionWeight::Uncalibrated), None);
}

#[test]
fn twice_a_day_for_a_week_is_the_meal_rate() {
    let feeds: Vec<FeedRow> = (0..14)
        .map(|meal| done(meal * 12 * 3600, 2, None))
        .collect();
    let now = at(13 * 12 * 3600 + 3600);

    let rate = rate(now, 7, &feeds, TEN_GRAMS).unwrap();

    // Two meals of two 10 g portions a day: 13 meals after the first,
    // over 157 hours.
    assert_eq!((rate.portions, rate.hours), (26, 157));
    assert!(
        rate.grams_per_day
            .is_some_and(|grams| grams.abs_diff(40) <= 1)
    );
    assert_eq!(super::rate(at(3600), 7, &feeds[..1], TEN_GRAMS), None);
}

#[test]
fn the_hopper_runs_out_at_the_rate() {
    let feeds = [done(3600, 3, None)];

    let since = since_refill(at(7200), &refill(0, 1_030), &feeds, TEN_GRAMS, Some(100));

    assert_eq!(since.portions, 3);
    let left = since.left.unwrap();
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
