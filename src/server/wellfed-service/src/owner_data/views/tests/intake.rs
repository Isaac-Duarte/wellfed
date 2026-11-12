//! A pet's intake under generated feeds and links: a shared calibrated
//! feeder splits evenly with nothing made or lost, an uncalibrated one's
//! portions stay whole, and every feed lands on one household day.

use jiff::tz::TimeZone;
use proptest::prelude::*;
use proptest::sample::select;
use uuid::Uuid;
use wellfed_api::{FeederPortions, PetLink};

use super::*;

proptest! {
    #[test]
    fn a_shared_feeder_splits_evenly_and_loses_nothing(
        feeds in proptest::collection::vec(feed(), 1..30),
        weight in calibration(),
        pets in 1..=4u128,
    ) {
        let links: Vec<PetLink> = (0..pets).map(|n| PetLink {
            pet: pet(n), feeder: KITCHEN, from: at(0), to: None,
        }).collect();
        let served = [Served { feeder: KITCHEN, weight, links: &links, feeds: &feeds }];
        let dates = days(date(2026, 11, 30), 30);
        let total = u64::from(Tally::of(&feeds).portions) * portion_mg(weight).unwrap();

        let shares: Vec<_> = (0..pets)
            .map(|n| intake(pet(n), None, &dates, &TimeZone::UTC, &served))
            .collect();

        let (feeders, first) = &shares[0];
        prop_assert_eq!(feeders[0].shared, pets > 1);
        for (_, days) in &shares {
            prop_assert_eq!(days, first, "every pet gets the same share");
        }
        let one: u64 = first.iter().map(|day| u64::from(day.weighed.unwrap().grams)).sum();
        let all = one * u64::try_from(pets).unwrap() * MG_PER_G;
        let slack = u64::try_from(feeds.len() + first.len()).unwrap() * 1_000 * 4;
        prop_assert!(all.abs_diff(total) <= slack, "{all} mg shared of {total} mg");
        for day in first {
            prop_assert!(!day.estimate || pets > 1, "one pet's intake is not an estimate");
            prop_assert!(day.unweighed.is_empty());
        }
    }

    /// An uncalibrated feeder's portions reach every pet it served whole,
    /// with no grams, calories or target.
    #[test]
    fn an_uncalibrated_feeder_is_counted_whole(
        feeds in proptest::collection::vec(feed(), 1..30),
        pets in 1..=4u128,
    ) {
        let links: Vec<PetLink> = (0..pets).map(|n| PetLink {
            pet: pet(n), feeder: KITCHEN, from: at(0), to: None,
        }).collect();
        let weight = PortionWeight::Uncalibrated;
        let served = [Served { feeder: KITCHEN, weight, links: &links, feeds: &feeds }];
        let dates = days(date(2026, 11, 30), 30);

        let (_, days) = intake(pet(pets - 1), Some(250), &dates, &TimeZone::UTC, &served);

        let portions: u32 = days.iter().flat_map(|day| &day.unweighed).map(|f| f.portions).sum();
        prop_assert_eq!(portions, Tally::of(&feeds).portions);
        for day in &days {
            prop_assert_eq!((day.weighed, day.target_percent, day.estimate), (None, None, false));
        }
    }

    #[test]
    fn every_feed_lands_on_one_household_day(
        feeds in feeds(),
        zone in select(vec!["UTC", "America/Denver", "Pacific/Kiritimati", "Asia/Kolkata"]),
    ) {
        let zone = wellfed_mirror::Zone::stored(zone, at(0)).unwrap().tz().clone();
        let links = [PetLink { pet: pet(0), feeder: KITCHEN, from: at(-86_400), to: None }];
        let served = [Served { feeder: KITCHEN, weight: TEN_GRAMS, links: &links, feeds: &feeds }];
        let dates = days(date(2026, 12, 2), 33);

        let (_, days) = intake(pet(0), None, &dates, &zone, &served);

        prop_assert_eq!(days.len(), 33);
        let grams: u64 = days.iter().map(|day| u64::from(day.weighed.unwrap().grams)).sum();
        prop_assert_eq!(grams, u64::from(Tally::of(&feeds).portions) * 10);
    }
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
        done(500, 2, Some(food(3600))),
        done(2000, 2, Some(food(3600))),
    ];
    let served = [Served {
        feeder: KITCHEN,
        weight: TEN_GRAMS,
        links: &links,
        feeds: &feeds,
    }];
    let dates = days(date(2026, 11, 1), 1);

    let (_, mine) = intake(pet(0), Some(36), &dates, &TimeZone::UTC, &served);
    let (_, theirs) = intake(pet(1), None, &dates, &TimeZone::UTC, &served);

    // 20 g each feed; the first is split, the second is pet 1's alone.
    let weighed = mine[0].weighed.unwrap();
    assert_eq!((weighed.grams, weighed.kcal), (10, 36));
    assert_eq!(mine[0].target_percent, Some(100));
    assert!(mine[0].estimate);
    assert_eq!(theirs[0].weighed.unwrap().grams, 30);
}

#[test]
fn one_uncalibrated_feeder_leaves_the_target_unknown() {
    let other = FeederId(Uuid::from_u128(2));
    let link = |feeder| PetLink {
        pet: pet(0),
        feeder,
        from: at(0),
        to: None,
    };
    let (kitchen, hall) = ([link(KITCHEN)], [link(other)]);
    let feeds = [done(500, 2, Some(food(3600)))];
    let hall_feeds = [FeedRow {
        feeder: other,
        ..done(600, 3, None)
    }];
    let served = [
        Served {
            feeder: KITCHEN,
            weight: TEN_GRAMS,
            links: &kitchen,
            feeds: &feeds,
        },
        Served {
            feeder: other,
            weight: PortionWeight::Uncalibrated,
            links: &hall,
            feeds: &hall_feeds,
        },
    ];
    let dates = days(date(2026, 11, 1), 1);

    let (_, days) = intake(pet(0), Some(72), &dates, &TimeZone::UTC, &served);

    assert_eq!(days[0].weighed.unwrap().kcal, 72);
    assert_eq!(days[0].target_percent, None);
    let unweighed = FeederPortions {
        feeder: other,
        portions: 3,
        uncertain_portions: 0,
    };
    assert_eq!(days[0].unweighed, [unweighed]);
}

/// A pet with a target and no feeder in the view: nothing is weighed, so
/// its share of the target is unknown, not 0%.
#[test]
fn a_pet_with_no_feeder_has_no_share_of_its_target() {
    let dates = [date(2026, 1, 1)];
    let (feeders, days) = intake(pet(0), Some(250), &dates, &TimeZone::UTC, &[]);
    assert!(feeders.is_empty());
    assert_eq!(days[0].weighed, None);
    assert_eq!(days[0].target_percent, None);
}
