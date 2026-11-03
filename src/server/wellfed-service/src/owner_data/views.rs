//! The derived views' arithmetic: food left since a refill, the feeder's
//! daily rate, a pet's intake by household day, and the feeds that went
//! wrong. Pure functions of typed rows, with "now" and the zone passed in,
//! so the property tests drive them without a store; [`super::derived`]
//! reads the rows.
//!
//! Portions are counted on every feeder; only the feeder's own calibration
//! turns them into grams, so an uncalibrated feeder's views stay in
//! portions. Weights are kept in milligrams and energy in
//! microkilocalories (a milligram of food at so many kcal per kg) until
//! the answer rounds them, so splitting and summing never lose more than a
//! milligram a share.

mod intake;
#[cfg(test)]
mod tests;

pub(crate) use intake::{Served, days, intake};
use jiff::{SignedDuration, Timestamp};
use wellfed_api::{
    FeedAnomaly, FeedOutcome, FeedRate, FeedReason, FeederId, Food, FoodLeft, MissedReason,
    PortionWeight, Refill, SinceRefill,
};
use wellfed_store::{Feed, FoodReading, Projected};

const DAY: SignedDuration = SignedDuration::from_hours(24);
const DAY_SECONDS: u64 = 86_400;
const MG_PER_G: u64 = 1_000;
/// Microkilocalories in a kilocalorie.
const UKCAL_PER_KCAL: u64 = 1_000_000;

/// A feed as the views read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FeedRow {
    pub(crate) feeder: FeederId,
    pub(crate) feed: u32,
    /// When the food dropped ([`dropped`]).
    pub(crate) at: Timestamp,
    pub(crate) requested: u8,
    pub(crate) completed: u8,
    pub(crate) outcome: FeedOutcome,
    pub(crate) reason: FeedReason,
    /// The food the feeder served at `at`; `None` before one was set.
    pub(crate) food: Option<Food>,
}

impl FeedRow {
    pub(crate) fn new(feeder: FeederId, feed: &Projected<Feed>, food: Option<Food>) -> Self {
        Self {
            feeder,
            feed: feed.value.feed,
            at: dropped(feed),
            requested: feed.value.requested,
            completed: feed.value.completed,
            outcome: feed.value.outcome,
            reason: feed.value.reason,
            food,
        }
    }
}

/// When a feed's food dropped: the feeder's own time when its clock was
/// valid, else when the server received the feed, which is later but never
/// earlier.
pub(crate) fn dropped(feed: &Projected<Feed>) -> Timestamp {
    feed.at.unwrap_or(feed.received)
}

/// A plan's meal the feeder reported missed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MissedRow {
    pub(crate) feeder: FeederId,
    pub(crate) plan: u8,
    pub(crate) due: Timestamp,
    pub(crate) reason: MissedReason,
}

/// How a feeder's portions are weighed: by its calibration (the portion's
/// weight in mg, as the server last read it), or not at all before one.
pub(crate) const fn portion_weight(calibration: Option<u32>) -> PortionWeight {
    match calibration {
        Some(portion_mg) => PortionWeight::Calibration { portion_mg },
        None => PortionWeight::Uncalibrated,
    }
}

/// One portion's weight in mg; `None` on an uncalibrated feeder.
fn portion_mg(weight: PortionWeight) -> Option<u64> {
    match weight {
        PortionWeight::Calibration { portion_mg } => Some(u64::from(portion_mg)),
        PortionWeight::Uncalibrated => None,
    }
}

/// Portions an uncertain feed may have dropped beyond those confirmed.
fn uncertain_portions(feed: &FeedRow) -> u8 {
    match feed.outcome {
        FeedOutcome::Uncertain => feed.requested.saturating_sub(feed.completed),
        FeedOutcome::Done | FeedOutcome::Cancelled | FeedOutcome::Expired | FeedOutcome::Failed => {
            0
        }
    }
}

/// The portions some feeds delivered: confirmed, and those uncertain feeds
/// may have dropped beyond them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Tally {
    portions: u32,
    uncertain: u32,
}

impl Tally {
    fn of<'a>(feeds: impl IntoIterator<Item = &'a FeedRow>) -> Self {
        let mut tally = Self::default();
        for feed in feeds {
            tally.portions = tally.portions.saturating_add(u32::from(feed.completed));
            let uncertain = u32::from(uncertain_portions(feed));
            tally.uncertain = tally.uncertain.saturating_add(uncertain);
        }
        tally
    }
}

/// `numerator / denominator` to the nearest, for a nonzero denominator,
/// held to a `u32`.
fn rounded(numerator: u64, denominator: u64) -> u32 {
    let quotient = numerator.saturating_add(denominator / 2) / denominator.max(1);
    u32::try_from(quotient).unwrap_or(u32::MAX)
}

/// What the feeder delivered over the `days` before `now`, confirmed
/// portions only, with grams a day by the calibration. The first feed in
/// the span starts the clock and is not counted, so a span of whole meal
/// intervals gives the meal rate whatever history the server holds; `None`
/// until the feeds span a day.
pub(crate) fn rate(
    now: Timestamp,
    days: u32,
    feeds: &[FeedRow],
    weight: PortionWeight,
) -> Option<FeedRate> {
    let start = before(now, days);
    let mut recent: Vec<&FeedRow> = feeds
        .iter()
        .filter(|feed| (start..=now).contains(&feed.at))
        .collect();
    recent.sort_by_key(|feed| feed.at);
    let (first, counted) = recent.split_first()?;

    let span = now.duration_since(first.at);
    if span < DAY {
        return None;
    }
    let portions = Tally::of(counted.iter().copied()).portions;
    let seconds = span.as_secs().unsigned_abs();
    let grams_per_day = portion_mg(weight).map(|mg| {
        let per_day = u128::from(portions) * u128::from(mg) * u128::from(DAY_SECONDS)
            / (u128::from(seconds) * u128::from(MG_PER_G));
        u32::try_from(per_day).unwrap_or(u32::MAX)
    });
    Some(FeedRate {
        portions,
        hours: rounded(seconds, 3600),
        grams_per_day,
    })
}

/// `days` whole days before `now`, or the earliest time there is.
pub(crate) fn before(now: Timestamp, days: u32) -> Timestamp {
    i32::try_from(days)
        .ok()
        .and_then(|days| DAY.checked_mul(days))
        .and_then(|span| now.checked_sub(span).ok())
        .unwrap_or(Timestamp::MIN)
}

/// What the feeds since `refill` delivered, and on a calibrated feeder what
/// the refill has left: what it put in, less what they delivered. A refill
/// the beam saw but nobody recorded resets nothing.
pub(crate) fn since_refill(
    now: Timestamp,
    refill: &Refill,
    feeds: &[FeedRow],
    weight: PortionWeight,
    grams_per_day: Option<u32>,
) -> SinceRefill {
    let tally = Tally::of(feeds.iter().filter(|feed| feed.at >= refill.at));
    let left = portion_mg(weight).map(|mg| food_left(now, refill, tally, mg, grams_per_day));
    SinceRefill {
        portions: tally.portions,
        uncertain_portions: tally.uncertain,
        left,
    }
}

fn food_left(
    now: Timestamp,
    refill: &Refill,
    tally: Tally,
    portion_mg: u64,
    grams_per_day: Option<u32>,
) -> FoodLeft {
    let used = u64::from(tally.portions).saturating_mul(portion_mg);
    let uncertain = u64::from(tally.uncertain).saturating_mul(portion_mg);
    let left = (u64::from(refill.grams) * MG_PER_G).saturating_sub(used);
    let least = left.saturating_sub(uncertain);

    let empty_at = grams_per_day.filter(|&rate| rate > 0).and_then(|rate| {
        let seconds =
            u128::from(left) * u128::from(DAY_SECONDS) / (u128::from(rate) * u128::from(MG_PER_G));
        now.checked_add(SignedDuration::from_secs(i64::try_from(seconds).ok()?))
            .ok()
    });
    FoodLeft {
        grams_used: rounded(used, MG_PER_G),
        grams_left: rounded(left, MG_PER_G),
        grams_left_least: rounded(least, MG_PER_G),
        empty_at,
    }
}

/// "Refilled?": the latest reading at which the food beam saw food again
/// after reading low, or the feeder counted a refill since the reading
/// before it in the same boot (its counter starts again at each boot).
/// `readings` are oldest first, all after the last recorded refill.
pub(crate) fn refilled_unrecorded(readings: &[FoodReading]) -> Option<Timestamp> {
    let mut seen = None;
    let mut low = false;
    let mut previous: Option<&FoodReading> = None;
    for reading in readings {
        let counted = previous.is_some_and(|before| {
            before.boot_count.is_some()
                && before.boot_count == reading.boot_count
                && reading.food.refilled > before.food.refilled
        });
        let beam = match reading.food.present {
            Some(true) => std::mem::replace(&mut low, false),
            Some(false) => {
                low = true;
                false
            }
            None => false,
        };

        if beam || counted {
            seen = Some(reading.received);
        }
        previous = Some(reading);
    }
    seen
}

/// What went wrong with a feed, if anything.
pub(crate) fn anomaly(feed: &FeedRow) -> Option<FeedAnomaly> {
    let (feeder, number, at) = (feed.feeder, feed.feed, feed.at);
    let (requested, completed) = (feed.requested, feed.completed);
    match feed.outcome {
        FeedOutcome::Uncertain => Some(FeedAnomaly::Uncertain {
            feeder,
            feed: number,
            at,
            requested,
            completed,
        }),
        FeedOutcome::Cancelled => None,
        FeedOutcome::Done | FeedOutcome::Expired | FeedOutcome::Failed => (completed < requested)
            .then_some(FeedAnomaly::Short {
                feeder,
                feed: number,
                at,
                requested,
                completed,
                outcome: feed.outcome,
                reason: feed.reason,
            }),
    }
}

/// The feeds that went wrong and the meals missed, oldest first.
pub(crate) fn anomalies<'a>(
    feeds: impl IntoIterator<Item = &'a FeedRow>,
    missed: &[MissedRow],
) -> Vec<FeedAnomaly> {
    let mut found: Vec<(Timestamp, FeedAnomaly)> = feeds
        .into_iter()
        .filter_map(|feed| Some((feed.at, anomaly(feed)?)))
        .collect();
    found.extend(missed.iter().map(|missed| {
        let anomaly = FeedAnomaly::Missed {
            feeder: missed.feeder,
            plan: missed.plan,
            due: missed.due,
            reason: missed.reason,
        };
        (missed.due, anomaly)
    }));

    found.sort_by_key(|(at, _)| *at);
    found.into_iter().map(|(_, anomaly)| anomaly).collect()
}
