//! The feeder's table of open alerts, as a client holds it. The feeder decides
//! every alert: a client displays them, and keeps no rules of its own.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

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
    /// Alerts a newer feeder build added, which this build cannot read:
    /// open, but not listed here. Update this client to see them.
    pub unread: u8,
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

impl OpenAlert {
    /// What makes this the same alert across reads and restarts, within one
    /// feeder, as the feeder's rules key it ([alerts]). A feed's number
    /// counts within the ring's epoch, which the view carries.
    ///
    /// [alerts]: ../../../../docs/firmware/alerts.md#identity
    pub const fn key(&self) -> AlertKey {
        let kind = self.alert.kind();
        match self.alert {
            Alert::MealFailed { feed, .. } | Alert::MealUncertain { feed, .. } => {
                AlertKey::Feed { feed }
            }
            Alert::MealMissed { plan, due, .. } | Alert::MealNoRecord { plan, due } => {
                AlertKey::Meal { plan, due }
            }
            Alert::ClientJoined { client, .. } | Alert::ClientRevoked { client, .. } => {
                AlertKey::Client {
                    kind,
                    client,
                    at: self.at,
                }
            }
            Alert::PowerWasOut | Alert::UpdateFailed { .. } | Alert::RolledBack => {
                AlertKey::Timed { kind, at: self.at }
            }
            Alert::RepeatedRestarts
            | Alert::NoClock
            | Alert::CannotFeed { .. }
            | Alert::FoodLow
            | Alert::FoodLikelyOut { .. }
            | Alert::DesiccantDue { .. }
            | Alert::BatteryBelowFloor
            | Alert::PlansStuck => AlertKey::Kind { kind },
        }
    }
}

impl Alert {
    pub const fn kind(&self) -> AlertKind {
        match self {
            Self::MealFailed { .. } => AlertKind::MealFailed,
            Self::MealUncertain { .. } => AlertKind::MealUncertain,
            Self::MealMissed { .. } => AlertKind::MealMissed,
            Self::MealNoRecord { .. } => AlertKind::MealNoRecord,
            Self::PowerWasOut => AlertKind::PowerWasOut,
            Self::UpdateFailed { .. } => AlertKind::UpdateFailed,
            Self::RolledBack => AlertKind::RolledBack,
            Self::ClientJoined { .. } => AlertKind::ClientJoined,
            Self::ClientRevoked { .. } => AlertKind::ClientRevoked,
            Self::RepeatedRestarts => AlertKind::RepeatedRestarts,
            Self::NoClock => AlertKind::NoClock,
            Self::CannotFeed { .. } => AlertKind::CannotFeed,
            Self::FoodLow => AlertKind::FoodLow,
            Self::FoodLikelyOut { .. } => AlertKind::FoodLikelyOut,
            Self::DesiccantDue { .. } => AlertKind::DesiccantDue,
            Self::BatteryBelowFloor => AlertKind::BatteryBelowFloor,
            Self::PlansStuck => AlertKind::PlansStuck,
        }
    }

    pub const fn level(&self) -> AlertLevel {
        self.kind().level()
    }
}

/// An alert's identity: equal keys are one alert. A feed's key spans its
/// outcomes, so an uncertain feed's second `Fed` replaces the first alert,
/// and a meal's spans a report of no record and the `Missed` that answers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(tag = "key", rename_all = "snake_case")]
pub enum AlertKey {
    Feed {
        feed: u32,
    },
    Meal {
        plan: u8,
        due: Timestamp,
    },
    /// With its time, so a later rejoin alerts again.
    Client {
        #[schema(value_type = String)]
        kind: AlertKind,
        #[schema(value_type = String)]
        client: Hex<4>,
        at: Option<Timestamp>,
    },
    Timed {
        #[schema(value_type = String)]
        kind: AlertKind,
        at: Option<Timestamp>,
    },
    /// A condition, present or absent.
    Kind {
        #[schema(value_type = String)]
        kind: AlertKind,
    },
}

/// An alert's kind, without what it says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertKind {
    MealFailed,
    MealUncertain,
    MealMissed,
    MealNoRecord,
    PowerWasOut,
    UpdateFailed,
    RolledBack,
    ClientJoined,
    ClientRevoked,
    RepeatedRestarts,
    NoClock,
    CannotFeed,
    FoodLow,
    FoodLikelyOut,
    DesiccantDue,
    BatteryBelowFloor,
    PlansStuck,
}

impl AlertKind {
    /// Every kind, in the enum's order.
    pub const ALL: [Self; 17] = [
        Self::MealFailed,
        Self::MealUncertain,
        Self::MealMissed,
        Self::MealNoRecord,
        Self::PowerWasOut,
        Self::UpdateFailed,
        Self::RolledBack,
        Self::ClientJoined,
        Self::ClientRevoked,
        Self::RepeatedRestarts,
        Self::NoClock,
        Self::CannotFeed,
        Self::FoodLow,
        Self::FoodLikelyOut,
        Self::DesiccantDue,
        Self::BatteryBelowFloor,
        Self::PlansStuck,
    ];

    /// How soon it wants the owner, as the feeder ranks it when room runs
    /// out.
    pub const fn level(self) -> AlertLevel {
        match self {
            Self::MealFailed
            | Self::MealUncertain
            | Self::MealMissed
            | Self::MealNoRecord
            | Self::NoClock
            | Self::CannotFeed
            | Self::FoodLikelyOut
            | Self::PlansStuck => AlertLevel::Urgent,
            Self::FoodLow
            | Self::DesiccantDue
            | Self::PowerWasOut
            | Self::BatteryBelowFloor
            | Self::UpdateFailed
            | Self::RolledBack
            | Self::RepeatedRestarts => AlertLevel::Soon,
            Self::ClientJoined | Self::ClientRevoked => AlertLevel::Security,
        }
    }

    /// Whether it is present or absent by its state, ending with it, rather
    /// than a report that lapses. Repeated restarts are tracked this way.
    pub const fn is_condition(self) -> bool {
        matches!(
            self,
            Self::RepeatedRestarts
                | Self::NoClock
                | Self::CannotFeed
                | Self::FoodLow
                | Self::FoodLikelyOut
                | Self::DesiccantDue
                | Self::BatteryBelowFloor
                | Self::PlansStuck
        )
    }
}

/// How soon an alert wants the owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertLevel {
    Urgent,
    Soon,
    Security,
}
