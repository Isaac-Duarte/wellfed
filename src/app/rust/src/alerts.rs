//! What the phone keeps of its feeders' alerts, in a file of its own
//! (`alerts.json` beside `wellfed.json`): the event cursor and epoch each
//! feeder's alerts were read to, the mutes, and the alerts the owner put
//! away. Its own file, so one that does not read is set aside on its own
//! ([`crate::stored`]) and can never keep the app from its feeders. Empty
//! until the alerts arrive; nothing opens it yet.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::api::error::BridgeError;
use crate::api::store::{SetAside, StoreKind};
use crate::stored::{self, Opened, Versioned};

/// The alert state the phone keeps.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlertState {}

impl Versioned for AlertState {
    const VERSION: u32 = 1;
}

/// The alert state at `path` as the app's start reads it, with the file set
/// aside if it did not read.
pub fn open_alerts(path: &Path) -> Result<(AlertState, Option<SetAside>), BridgeError> {
    Ok(
        match stored::open(path, StoreKind::Alerts, |_| AlertState::default())? {
            Opened::Absent => (AlertState::default(), None),
            Opened::Read(state) => (state, None),
            Opened::SetAside { value, set_aside } => (value, Some(set_aside)),
        },
    )
}

pub fn save_alerts(path: &Path, state: &AlertState) -> Result<(), BridgeError> {
    stored::write(path, state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_damaged_alerts_file_is_set_aside_on_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("alerts.json");
        assert_eq!(open_alerts(&path).unwrap(), (AlertState::default(), None));

        save_alerts(&path, &AlertState::default()).unwrap();
        assert_eq!(open_alerts(&path).unwrap(), (AlertState::default(), None));

        std::fs::write(&path, b"{\"version\": 1, \"data\": {\"mutes\": 3}}").unwrap();
        let (state, set_aside) = open_alerts(&path).unwrap();
        assert_eq!(state, AlertState::default());
        assert_eq!(set_aside.map(|aside| aside.kind), Some(StoreKind::Alerts));
        assert!(dir.path().join("alerts.json.damaged-1").exists());
    }
}
