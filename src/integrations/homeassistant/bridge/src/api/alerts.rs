//! A feeder's open alerts as Home Assistant shows them: a sensor of what is
//! open, and an event for each alert that opens or clears. The feeder
//! decides every alert; which opened and which cleared is
//! `wellfed_client::alerts`'s judgement, the phone's own, never this
//! integration's.

use wellfed_client::alerts::{Notice, NoticeKind, Tracked};
use wellfed_model as model;

view_enum! {
    /// What an alert is about, as the app's tracker names it. `Unreachable`
    /// is a relay's judgement that it can't reach the feeder, which a
    /// direct session never makes: its Connected sensor says so.
    AlertKind {
        MealFailed => "meal_failed",
        MealUncertain => "meal_uncertain",
        MealMissed => "meal_missed",
        MealNoRecord => "meal_no_record",
        PowerWasOut => "power_was_out",
        UpdateFailed => "update_failed",
        RolledBack => "rolled_back",
        ClientJoined => "client_joined",
        ClientRevoked => "client_revoked",
        RepeatedRestarts => "repeated_restarts",
        NoClock => "no_clock",
        CannotFeed => "cannot_feed",
        FoodLow => "food_low",
        FoodLikelyOut => "food_likely_out",
        DesiccantDue => "desiccant_due",
        BatteryBelowFloor => "battery_below_floor",
        PlansStuck => "plans_stuck",
        Unreachable => "unreachable",
    }
}

impl From<NoticeKind> for AlertKind {
    fn from(kind: NoticeKind) -> Self {
        match kind {
            NoticeKind::MealFailed => Self::MealFailed,
            NoticeKind::MealUncertain => Self::MealUncertain,
            NoticeKind::MealMissed => Self::MealMissed,
            NoticeKind::MealNoRecord => Self::MealNoRecord,
            NoticeKind::PowerWasOut => Self::PowerWasOut,
            NoticeKind::UpdateFailed => Self::UpdateFailed,
            NoticeKind::RolledBack => Self::RolledBack,
            NoticeKind::ClientJoined => Self::ClientJoined,
            NoticeKind::ClientRevoked => Self::ClientRevoked,
            NoticeKind::RepeatedRestarts => Self::RepeatedRestarts,
            NoticeKind::NoClock => Self::NoClock,
            NoticeKind::CannotFeed => Self::CannotFeed,
            NoticeKind::FoodLow => Self::FoodLow,
            NoticeKind::FoodLikelyOut => Self::FoodLikelyOut,
            NoticeKind::DesiccantDue => Self::DesiccantDue,
            NoticeKind::BatteryBelowFloor => Self::BatteryBelowFloor,
            NoticeKind::PlansStuck => Self::PlansStuck,
            NoticeKind::Unreachable => Self::Unreachable,
        }
    }
}

view_enum! {
    /// How soon an alert wants the owner.
    AlertLevel {
        Urgent => "urgent",
        Soon => "soon",
        Security => "security",
    }
}

impl From<model::AlertLevel> for AlertLevel {
    fn from(level: model::AlertLevel) -> Self {
        match level {
            model::AlertLevel::Urgent => Self::Urgent,
            model::AlertLevel::Soon => Self::Soon,
            model::AlertLevel::Security => Self::Security,
        }
    }
}

/// One alert.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(frozen, get_all, eq, module = "wellfed_homeassistant")
)]
pub struct Alert {
    /// The alert's identity on its feeder, the same in the event that opens
    /// it and the one that clears it.
    pub key: String,
    pub kind: AlertKind,
    pub level: AlertLevel,
    /// The feeder's time for it, Unix milliseconds; `None` without a valid
    /// clock then.
    pub at_ms: Option<i64>,
}

impl From<&Tracked> for Alert {
    fn from(tracked: &Tracked) -> Self {
        let kind = tracked.notice.kind();
        let at = match tracked.notice {
            Notice::Alert { open } => open.at,
            Notice::Unreachable { since } => Some(since),
        };
        Self {
            key: tracked.key.text(),
            kind: kind.into(),
            level: kind.level().into(),
            at_ms: at.map(jiff::Timestamp::as_millisecond),
        }
    }
}

/// The feeder's alerts after a read: what is open, and what opened and
/// cleared since the read before.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    feature = "python",
    pyo3::pyclass(frozen, get_all, eq, module = "wellfed_homeassistant")
)]
pub struct Alerts {
    /// Open now, oldest first.
    pub open: Vec<Alert>,
    pub opened: Vec<Alert>,
    pub cleared: Vec<Alert>,
    /// Alerts the feeder holds that this integration is too old to read.
    pub unread: u8,
}
