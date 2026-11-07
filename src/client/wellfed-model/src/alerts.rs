//! The feeder's table of open alerts, as a client holds it. The feeder decides
//! every alert: a client displays them, and keeps no rules of its own.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::event::{ClientRole, EnrollPath, FeedOutcome, FeedReason, MissedReason, Source};
use crate::{Blocked, Generation, Hex, UpdateStop};

/// The table, as one read of it found it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertsView {
    /// The history a feed's number counts in: with it, a feed's alert has
    /// the same identity across restarts.
    pub epoch: u32,
    /// Changes when anything here does. The status carries it too.
    pub generation: Generation,
    /// A report left the table for want of room, or the boot read missed some
    /// of the ring, so an absent one may not have ended.
    pub more: bool,
    /// Thresholds are running again after a boot, or a reading an alert needs
    /// is unknown, so an absent condition may not have ended.
    pub settling: bool,
    /// Oldest first.
    pub alerts: Vec<OpenAlert>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAlert {
    /// A report's own time, or when a condition opened; `None` if the
    /// feeder's clock was not valid then.
    pub at: Option<Timestamp>,
    pub alert: Alert,
}

/// What is wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Alert {
    /// A feed failed, or never started. `plan` is its slot, if a plan asked.
    MealFailed {
        feed: u32,
        plan: Option<u8>,
        requested: u8,
        completed: u8,
        outcome: FeedOutcome,
        reason: FeedReason,
    },
    /// A feed may have dispensed anything up to `requested`.
    MealUncertain {
        feed: u32,
        plan: Option<u8>,
        requested: u8,
        completed: u8,
        reason: FeedReason,
    },
    /// The occurrence did not run, and never will.
    MealMissed {
        plan: u8,
        due: Timestamp,
        reason: MissedReason,
    },
    /// The occurrence is past due and the feeder's schedule has no record of
    /// it.
    MealNoRecord {
        plan: u8,
        due: Timestamp,
    },
    PowerWasOut,
    UpdateFailed {
        stop: UpdateStop,
    },
    RolledBack,
    ClientJoined {
        client: Hex<4>,
        role: ClientRole,
        path: EnrollPath,
    },
    ClientRevoked {
        client: Hex<4>,
        by: Source,
    },
    RepeatedRestarts,
    NoClock,
    CannotFeed {
        reason: Blocked,
    },
    FoodLow,
    /// There is no empty reading: `confirmed` portions have gone since the
    /// food beam read low, about what lay below it. Word it as likely.
    FoodLikelyOut {
        confirmed: u8,
    },
    /// Days left of the desiccant's period: 0 or below.
    DesiccantDue {
        days: i32,
    },
    BatteryBelowFloor,
    /// With a valid clock, the plans have stayed out of reach, or a change to
    /// them has never finished, so nothing can say whether meals ran.
    PlansStuck,
}
