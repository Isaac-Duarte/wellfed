//! A pet's intake by household day: each feed lands on the day it dropped
//! in the household's zone, split evenly among the pets its feeder served
//! then, and counted in calories by the food it served.

use std::collections::{BTreeMap, BTreeSet};

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use wellfed_api::{FeederId, IntakeDay, IntakeFeeder, PetId, PetLink, PortionWeight};

use super::{FeedRow, MG_PER_G, Tally, UKCAL_PER_KCAL, rounded};

/// The `count` household days ending `today`, oldest first.
pub(crate) fn days(today: Date, count: u32) -> Vec<Date> {
    let dates: BTreeSet<Date> = (0..count)
        .map(|back| today.saturating_sub(i64::from(back).days()))
        .collect();
    dates.into_iter().collect()
}

/// A feeder an intake reads: every pet's links to it over the view's days
/// (so a shared feed is split among the pets it served then), and its
/// feeds.
pub(crate) struct Served<'a> {
    pub(crate) feeder: FeederId,
    pub(crate) weight: PortionWeight,
    pub(crate) links: &'a [PetLink],
    pub(crate) feeds: &'a [FeedRow],
}

/// The pets a feeder served at `at`.
fn pets_at(links: &[PetLink], at: Timestamp) -> BTreeSet<PetId> {
    links
        .iter()
        .filter(|link| link.from <= at && link.to.is_none_or(|to| at < to))
        .map(|link| link.pet)
        .collect()
}

/// One day's sums, before rounding.
#[derive(Clone, Copy, Debug, Default)]
struct DaySum {
    mg: u64,
    ukcal: u64,
    uncertain_mg: u64,
    unknown_food_mg: u64,
    unweighed: u32,
    estimate: bool,
}

/// What a pet ate on `dates` (household days in `zone`), from the feeders
/// that served it, against its target; and those feeders.
pub(crate) fn intake(
    pet: PetId,
    daily_kcal: Option<u32>,
    dates: &[Date],
    zone: &TimeZone,
    served: &[Served<'_>],
) -> (Vec<IntakeFeeder>, Vec<IntakeDay>) {
    let mut sums: BTreeMap<Date, DaySum> = dates
        .iter()
        .map(|&date| (date, DaySum::default()))
        .collect();
    let mut feeders = Vec::new();
    for source in served {
        if !source.links.iter().any(|link| link.pet == pet) {
            continue;
        }
        let mut shared = false;
        for feed in source.feeds {
            let pets = pets_at(source.links, feed.at);
            let Some(sum) = sums.get_mut(&zone.to_datetime(feed.at).date()) else {
                continue;
            };
            if !pets.contains(&pet) {
                continue;
            }
            let share = u64::try_from(pets.len()).unwrap_or(u64::MAX);
            shared |= share > 1;
            add_share(sum, feed, source.weight, share);
        }
        feeders.push(IntakeFeeder {
            feeder: source.feeder,
            weight: source.weight,
            shared,
        });
    }

    let days = sums
        .into_iter()
        .map(|(date, sum)| IntakeDay {
            date,
            grams: rounded(sum.mg, MG_PER_G),
            kcal: rounded(sum.ukcal, UKCAL_PER_KCAL),
            target_percent: daily_kcal
                .filter(|&target| target > 0)
                .map(|target| rounded(sum.ukcal * 100, u64::from(target) * UKCAL_PER_KCAL)),
            estimate: sum.estimate,
            uncertain_grams: rounded(sum.uncertain_mg, MG_PER_G),
            unknown_food_grams: rounded(sum.unknown_food_mg, MG_PER_G),
            unweighed_portions: sum.unweighed,
        })
        .collect();
    (feeders, days)
}

/// Add one pet's even share of `feed` (of `share` pets) to its day.
fn add_share(sum: &mut DaySum, feed: &FeedRow, weight: PortionWeight, share: u64) {
    let tally = Tally::of([feed], weight);
    let mg = tally.mg / share;

    sum.mg = sum.mg.saturating_add(mg);
    sum.uncertain_mg = sum.uncertain_mg.saturating_add(tally.uncertain_mg / share);
    sum.unweighed = sum.unweighed.saturating_add(tally.unweighed);
    sum.estimate |= share > 1;
    match &feed.food {
        Some(food) => sum.ukcal = sum.ukcal.saturating_add(mg * u64::from(food.kcal_per_kg)),
        None => sum.unknown_food_mg = sum.unknown_food_mg.saturating_add(mg),
    }
}
