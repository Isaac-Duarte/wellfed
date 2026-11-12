//! A pet's intake by household day: each feed lands on the day it dropped
//! in the household's zone. A calibrated feeder's feed is weighed, split
//! evenly among the pets the feeder served then, and counted in calories
//! by the food it served. An uncalibrated feeder's portions cannot be
//! split by weight, so they are reported whole, a feeder at a time.

use std::collections::{BTreeMap, BTreeSet};

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use wellfed_api::{
    FeederId, FeederPortions, IntakeDay, IntakeFeeder, PetId, PetLink, PortionWeight, WeighedIntake,
};

use super::{FeedRow, MG_PER_G, Tally, UKCAL_PER_KCAL, portion_mg, rounded};

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
#[derive(Clone, Debug, Default)]
struct DaySum {
    mg: u64,
    ukcal: u64,
    uncertain_mg: u64,
    unknown_food_mg: u64,
    estimate: bool,
    /// The uncalibrated feeders' portions, by feeder.
    unweighed: BTreeMap<FeederId, Tally>,
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
            match portion_mg(source.weight) {
                Some(mg) => add_share(sum, feed, mg, share),
                None => add_whole(sum, feed, source.feeder),
            }
        }
        feeders.push(IntakeFeeder {
            feeder: source.feeder,
            weight: source.weight,
            shared,
        });
    }

    let weights = || feeders.iter().map(|feeder| portion_mg(feeder.weight));
    let any_weighed = weights().any(|mg| mg.is_some());
    let all_weighed = weights().all(|mg| mg.is_some());
    let days = sums
        .into_iter()
        // With no feeder in the view nothing is weighed, so the share of
        // the target is as unknown as the intake.
        .map(|(date, sum)| {
            let target = daily_kcal.filter(|_| any_weighed && all_weighed);
            day(date, sum, any_weighed, target)
        })
        .collect();
    (feeders, days)
}

/// The day as the view reports it: weighed when any of the pet's feeders
/// is calibrated, and against the target only when all are.
fn day(date: Date, sum: DaySum, weighed: bool, target: Option<u32>) -> IntakeDay {
    let target_percent = target.filter(|&target| target > 0).map(|target| {
        let whole = u64::from(target) * UKCAL_PER_KCAL;
        rounded(sum.ukcal.saturating_mul(100), whole)
    });
    let unweighed = sum
        .unweighed
        .into_iter()
        .map(|(feeder, tally)| FeederPortions {
            feeder,
            portions: tally.portions,
            uncertain_portions: tally.uncertain,
        })
        .collect();

    IntakeDay {
        date,
        weighed: weighed.then(|| WeighedIntake {
            grams: rounded(sum.mg, MG_PER_G),
            kcal: rounded(sum.ukcal, UKCAL_PER_KCAL),
            uncertain_grams: rounded(sum.uncertain_mg, MG_PER_G),
            unknown_food_grams: rounded(sum.unknown_food_mg, MG_PER_G),
        }),
        target_percent,
        estimate: sum.estimate,
        unweighed,
    }
}

/// Add one pet's even share of `feed` (of `share` pets), at `portion_mg`
/// a portion, to its day.
fn add_share(sum: &mut DaySum, feed: &FeedRow, portion_mg: u64, share: u64) {
    let tally = Tally::of([feed]);
    let mg = u64::from(tally.portions).saturating_mul(portion_mg) / share;
    let uncertain_mg = u64::from(tally.uncertain).saturating_mul(portion_mg) / share;

    sum.mg = sum.mg.saturating_add(mg);
    sum.uncertain_mg = sum.uncertain_mg.saturating_add(uncertain_mg);
    sum.estimate |= share > 1;
    match &feed.food {
        Some(food) => {
            let ukcal = mg.saturating_mul(u64::from(food.kcal_per_kg));
            sum.ukcal = sum.ukcal.saturating_add(ukcal);
        }
        None => sum.unknown_food_mg = sum.unknown_food_mg.saturating_add(mg),
    }
}

/// Add an uncalibrated feeder's `feed` to its day whole.
fn add_whole(sum: &mut DaySum, feed: &FeedRow, feeder: FeederId) {
    let tally = Tally::of([feed]);
    let kept = sum.unweighed.entry(feeder).or_default();

    kept.portions = kept.portions.saturating_add(tally.portions);
    kept.uncertain = kept.uncertain.saturating_add(tally.uncertain);
}
