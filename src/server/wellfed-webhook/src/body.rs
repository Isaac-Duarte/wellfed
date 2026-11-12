//! The JSON each delivery POSTs: small and frozen from the beta. A newer
//! server only adds fields, so a receiver reads what it knows and ignores
//! the rest; `version` changes only if a field's meaning ever must.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use wellfed_api::FeederId;
use wellfed_client::alerts::{Notice, NoticeKey, NoticeKind};
use wellfed_model::{Alert, AlertLevel};

/// The body's layout.
pub const VERSION: u32 = 1;

/// One delivery: an alert opened or cleared on one feeder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delivery {
    pub version: u32,
    /// The same for every try of one change, so a receiver that saw a try
    /// whose answer was lost can tell the next one is no new change.
    pub id: String,
    pub event: Transition,
    /// When the server saw the change: an opening's when it first judged
    /// the alert open, a clear's when it first tried to send it, which a
    /// change ahead of it in the queue can delay.
    pub time: Timestamp,
    pub feeder: FeederRef,
    pub alert: AlertRef,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transition {
    Opened,
    Cleared,
}

impl Transition {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Opened => "opened",
            Self::Cleared => "cleared",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeederRef {
    pub id: FeederId,
    /// What the household calls it.
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertRef {
    /// The alert's identity on its feeder: an opened and a cleared delivery
    /// of one alert carry the same key.
    pub key: String,
    pub kind: NoticeKind,
    pub level: AlertLevel,
    /// One plain English sentence, for a receiver that shows it as it is.
    pub words: String,
    /// The feeder's time for it (a report's own, or when a condition
    /// began, or since when the server can't reach the feeder); `None`
    /// without a valid clock then.
    pub at: Option<Timestamp>,
}

impl AlertRef {
    pub(crate) fn new(key: &NoticeKey, notice: &Notice) -> Self {
        let kind = notice.kind();
        let at = match notice {
            Notice::Alert { open } => open.at,
            Notice::Unreachable { since } => Some(*since),
        };
        Self {
            key: key.text(),
            kind,
            level: kind.level(),
            words: words(notice),
            at,
        }
    }
}

/// What a notice says, in a sentence.
fn words(notice: &Notice) -> String {
    let alert = match notice {
        Notice::Unreachable { .. } => return "The server can't reach the feeder.".to_owned(),
        Notice::Alert { open } => open.alert,
    };
    match alert {
        Alert::MealFailed {
            requested,
            completed,
            ..
        } => format!("A meal failed: {completed} of {requested} portions dispensed."),
        Alert::MealUncertain { requested, .. } => {
            format!("A meal may not have dispensed: anything up to {requested} portions.")
        }
        Alert::MealMissed { .. } => "A planned meal was missed.".to_owned(),
        Alert::MealNoRecord { .. } => "A planned meal has no record.".to_owned(),
        Alert::PowerWasOut => "The power was out.".to_owned(),
        Alert::UpdateFailed { .. } => "An update failed.".to_owned(),
        Alert::RolledBack => "The feeder went back to its earlier firmware.".to_owned(),
        Alert::ClientJoined { client, .. } => format!("A new client joined ({client})."),
        Alert::ClientRevoked { client, .. } => format!("A client was removed ({client})."),
        Alert::RepeatedRestarts => "The feeder keeps restarting.".to_owned(),
        Alert::NoClock => {
            "The feeder doesn't know the local time, so it holds its plans.".to_owned()
        }
        Alert::CannotFeed { .. } => "The feeder can't feed.".to_owned(),
        Alert::FoodLow => "Food is low.".to_owned(),
        Alert::FoodLikelyOut { .. } => "Food has likely run out.".to_owned(),
        Alert::DesiccantDue { .. } => "The desiccant is due for a change.".to_owned(),
        Alert::BatteryBelowFloor => "The battery needs replacing.".to_owned(),
        Alert::PlansStuck => "The feeding plans are stuck, so no meal can be judged.".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use wellfed_model::OpenAlert;

    use super::*;

    #[test]
    fn food_low_reads_plainly() {
        let open = OpenAlert {
            at: None,
            alert: Alert::FoodLow,
        };
        let notice = Notice::Alert { open };
        assert_eq!(words(&notice), "Food is low.");
    }
}
