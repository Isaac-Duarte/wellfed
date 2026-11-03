//! Derived views: what the hopper has left, what each pet ate against its
//! target, and the feeds that went wrong. The server works them out from
//! the feeds it mirrors and the owner data; none of it is stored.
//!
//! Every weight is an estimate. A portion weighs what the feeder's own
//! calibration says when the server has read one, and otherwise a twelfth
//! of a cup of the food the feed served. The feeder cannot tell which pet
//! ate, so a feeder several pets share has its intake split evenly.

use jiff::Timestamp;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use wellfed_model::{FeedOutcome, FeedReason, MissedReason};

use crate::{FeederId, PetId};

/// The days an intake view covers when the request does not say.
pub const INTAKE_DAYS: u32 = 7;
/// The most days one intake view covers.
pub const INTAKE_DAYS_MAX: u32 = 31;
/// The days a feeder's food view looks back for its rate and anomalies.
pub const FOOD_VIEW_DAYS: u32 = 7;

/// How a view weighed a feeder's portions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PortionWeight {
    /// The feeder's own calibration, as the server last read it, for every
    /// feed: the feeder keeps one calibration, not its history.
    Calibration { portion_mg: u32 },
    /// No calibration read: each portion is taken as 1/12 cup of the food
    /// the feed served (its `grams_per_cup`). A feed with no food set is
    /// not weighed.
    TwelfthCup,
}

/// What the hopper has left by the count since the last refill: what that
/// refill put in, less what the feeder delivered since. Food left before
/// the refill is not known, so it is not counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FoodLeft {
    /// Portions the feedback sensor confirmed since the refill.
    pub portions: u32,
    /// Portions uncertain feeds may have dropped beyond those confirmed.
    pub uncertain_portions: u32,
    /// Confirmed portions with no weight: no calibration and no food set.
    pub unweighed_portions: u32,
    pub grams_used: u32,
    /// The refill's grams less those used, at least 0.
    pub grams_left: u32,
    /// What is left should every uncertain portion have dropped.
    pub grams_left_least: u32,
    /// When `grams_left` runs out at `grams_per_day`; `None` without a rate.
    pub empty_at: Option<Timestamp>,
}

/// A feed that went wrong, or a plan's meal that never ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FeedAnomaly {
    /// A plan's occurrence did not run, and never will.
    Missed {
        feeder: FeederId,
        plan: u8,
        due: Timestamp,
        reason: MissedReason,
    },
    /// The feeder stopped without recording a result: anything up to
    /// `requested` may have dropped, `completed` of it confirmed.
    Uncertain {
        feeder: FeederId,
        feed: u32,
        at: Timestamp,
        requested: u8,
        completed: u8,
    },
    /// The feed ended with fewer portions than it asked for (a cancel is a
    /// person's choice, so it is not one).
    Short {
        feeder: FeederId,
        feed: u32,
        at: Timestamp,
        requested: u8,
        completed: u8,
        outcome: FeedOutcome,
        reason: FeedReason,
    },
}

/// `GET /pets/{id}/intake`'s days.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct IntakeQuery {
    /// The household days to cover, ending today (1-31, default 7).
    pub days: Option<u32>,
}

/// What a pet ate, a household day at a time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PetIntake {
    pub pet: PetId,
    /// The zone the days are in: the household's, or `UTC` before one.
    pub zone: String,
    /// The pet's daily target, as it stands now.
    pub daily_kcal: Option<u32>,
    /// The feeders that served the pet during these days.
    pub feeders: Vec<IntakeFeeder>,
    /// Oldest first, today last.
    pub days: Vec<IntakeDay>,
    /// What went wrong on those feeders during these days, oldest first.
    pub anomalies: Vec<FeedAnomaly>,
}

/// A feeder an intake view counted, and how.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct IntakeFeeder {
    pub feeder: FeederId,
    pub weight: PortionWeight,
    /// It served other pets too during these days, so its intake is split
    /// between them as an estimate.
    pub shared: bool,
}

/// One household day of a pet's intake.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct IntakeDay {
    pub date: Date,
    /// What the feeders delivered for the pet: confirmed portions only.
    pub grams: u32,
    /// Of `grams` with a known food.
    pub kcal: u32,
    /// `kcal` as a percentage of the daily target; `None` without one.
    pub target_percent: Option<u32>,
    /// Some of it came from a feeder other pets share, split evenly: the
    /// feeder cannot tell who ate.
    pub estimate: bool,
    /// Grams more uncertain feeds may have dropped for the pet.
    pub uncertain_grams: u32,
    /// Of `grams`, those dropped with no food set: no calories counted.
    pub unknown_food_grams: u32,
    /// Confirmed portions on the pet's feeders that could not be weighed
    /// (no calibration, no food set), counted whole rather than split.
    pub unweighed_portions: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_portion_weight_names_its_kind() {
        let calibration = PortionWeight::Calibration { portion_mg: 9200 };

        assert_eq!(
            serde_json::to_string(&calibration).unwrap(),
            r#"{"kind":"calibration","portion_mg":9200}"#
        );
        assert_eq!(
            serde_json::to_string(&PortionWeight::TwelfthCup).unwrap(),
            r#"{"kind":"twelfth_cup"}"#
        );
    }
}
