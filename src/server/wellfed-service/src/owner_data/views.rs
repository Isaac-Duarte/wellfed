//! The derived views' arithmetic: food left since a refill, the feeder's
//! daily rate, a pet's intake by household day, and the feeds that went
//! wrong. Pure functions of typed rows, with "now" and the zone passed in,
//! so the property tests drive them without a store; [`super::derived`]
//! reads the rows.
//!
//! Weights are kept in milligrams and energy in microkilocalories (a
//! milligram of food at so many kcal per kg) until the answer rounds them,
//! so splitting and summing never lose more than a milligram a share.

mod intake;
#[cfg(test)]
mod tests;

pub(crate) use intake::{Served, days, intake};
use jiff::{SignedDuration, Timestamp};
use wellfed_api::{
    FeedAnomaly, FeedOutcome, FeedReason, FeederId, Food, FoodLeft, MissedReason, PortionWeight,
    Refill,
};
use wellfed_store::{Feed, FoodReading, Projected};

const DAY: SignedDuration = SignedDuration::from_hours(24);
const MG_PER_G: u64 = 1_000;
/// Microkilocalories in a kilocalorie.
const UKCAL_PER_KCAL: u64 = 1_000_000;
/// A portion is 1/12 cup on a feeder with no calibration (provisional).
const PORTIONS_PER_CUP: u64 = 12;

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
/// weight in mg, as the server last read it) when it has one.
pub(crate) const fn portion_weight(calibration: Option<u32>) -> PortionWeight {
    match calibration {
        Some(portion_mg) => PortionWeight::Calibration { portion_mg },
        None => PortionWeight::TwelfthCup,
    }
}

/// One portion's weight in mg, or `None` when neither the calibration nor
/// a food says.
fn portion_mg(weight: PortionWeight, food: Option<&Food>) -> Option<u64> {
    match weight {
        PortionWeight::Calibration { portion_mg } => Some(u64::from(portion_mg)),
        PortionWeight::TwelfthCup => {
            food.map(|food| u64::from(food.grams_per_cup) * MG_PER_G / PORTIONS_PER_CUP)
        }
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

/// What some feeds delivered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Tally {
    portions: u32,
    uncertain: u32,
    unweighed: u32,
    mg: u64,
    uncertain_mg: u64,
}

impl Tally {
    fn of<'a>(feeds: impl IntoIterator<Item = &'a FeedRow>, weight: PortionWeight) -> Self {
        let mut tally = Self::default();
        for feed in feeds {
            let (confirmed, uncertain) = (feed.completed, uncertain_portions(feed));
            tally.portions = tally.portions.saturating_add(u32::from(confirmed));
            tally.uncertain = tally.uncertain.saturating_add(u32::from(uncertain));
            match portion_mg(weight, feed.food.as_ref()) {
                Some(mg) => {
                    tally.mg = tally.mg.saturating_add(mg * u64::from(confirmed));
                    tally.uncertain_mg =
                        tally.uncertain_mg.saturating_add(mg * u64::from(uncertain));
                }
                None => tally.unweighed = tally.unweighed.saturating_add(u32::from(confirmed)),
            }
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

/// What the feeder delivered a day over the `days` before `now`, in grams,
/// confirmed portions only. The first feed in the span starts the clock
/// and is not counted, so a span of whole meal intervals gives the meal
/// rate whatever the history the server holds; `None` until the feeds span
/// a day.
pub(crate) fn grams_per_day(
    now: Timestamp,
    days: u32,
    feeds: &[FeedRow],
    weight: PortionWeight,
) -> Option<u32> {
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
    let mg = Tally::of(counted.iter().copied(), weight).mg;
    let seconds = u128::from(span.as_secs().unsigned_abs());
    let per_day = u128::from(mg) * u128::from(DAY.as_secs().unsigned_abs())
        / (seconds * u128::from(MG_PER_G));
    Some(u32::try_from(per_day).unwrap_or(u32::MAX))
}

/// `days` whole days before `now`, or the earliest time there is.
pub(crate) fn before(now: Timestamp, days: u32) -> Timestamp {
    i32::try_from(days)
        .ok()
        .and_then(|days| DAY.checked_mul(days))
        .and_then(|span| now.checked_sub(span).ok())
        .unwrap_or(Timestamp::MIN)
}

/// What the hopper has left by the count since `refill`: what it put in,
/// less what the feeds since delivered. A refill the beam saw but nobody
/// recorded resets nothing.
pub(crate) fn food_left(
    now: Timestamp,
    refill: &Refill,
    feeds: &[FeedRow],
    weight: PortionWeight,
    grams_per_day: Option<u32>,
) -> FoodLeft {
    let tally = Tally::of(feeds.iter().filter(|feed| feed.at >= refill.at), weight);
    let left = (u64::from(refill.grams) * MG_PER_G).saturating_sub(tally.mg);
    let least = left.saturating_sub(tally.uncertain_mg);

    let empty_at = grams_per_day.filter(|&rate| rate > 0).and_then(|rate| {
        let seconds = u128::from(left) * u128::from(DAY.as_secs().unsigned_abs())
            / (u128::from(rate) * u128::from(MG_PER_G));
        now.checked_add(SignedDuration::from_secs(i64::try_from(seconds).ok()?))
            .ok()
    });
    FoodLeft {
        portions: tally.portions,
        uncertain_portions: tally.uncertain,
        unweighed_portions: tally.unweighed,
        grams_used: rounded(tally.mg, MG_PER_G),
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
