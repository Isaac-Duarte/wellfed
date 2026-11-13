//! What each request from Dart is to the router: a read, which goes as it
//! is, or a write, which carries a key and is pinned before it goes. The pin
//! keeps the write's kind alone (`PinnedWrite`), so no secret (a Wi-Fi
//! passphrase) is ever written down, and nothing is ever sent again from a
//! pin.

use crate::api::feeder::FeederRequest;
use crate::api::router::WriteKind;

/// The kind of write `request` is, or `None` for a read.
pub(crate) const fn write_kind(request: &FeederRequest) -> Option<WriteKind> {
    match request {
        FeederRequest::GetSettings
        | FeederRequest::GetTime
        | FeederRequest::ListClients
        | FeederRequest::Stats { .. }
        | FeederRequest::ReadTrace { .. }
        | FeederRequest::GetCalibration
        | FeederRequest::GetPlans
        | FeederRequest::GetCall
        | FeederRequest::UpdateState
        | FeederRequest::TailLog { .. } => None,
        FeederRequest::PutSettings { .. } => Some(WriteKind::Settings),
        FeederRequest::SetLock { .. } => Some(WriteKind::Lock),
        FeederRequest::ResetDesiccant => Some(WriteKind::Desiccant),
        FeederRequest::SkipMeal { .. } | FeederRequest::UnskipMeal { .. } => Some(WriteKind::Skip),
        FeederRequest::EditPlans { .. } => Some(WriteKind::Plans),
        FeederRequest::PutCalibration { .. } => Some(WriteKind::Calibration),
        FeederRequest::UploadCall { .. } | FeederRequest::DeleteCall | FeederRequest::PlayCall => {
            Some(WriteKind::Call)
        }
        FeederRequest::PutWifi { .. } | FeederRequest::ForgetWifi { .. } => Some(WriteKind::Wifi),
        FeederRequest::Play { .. } => Some(WriteKind::Play),
        FeederRequest::SyncClock | FeederRequest::PutTime { .. } => Some(WriteKind::Clock),
        FeederRequest::CreateInvite { .. } => Some(WriteKind::Invite),
        FeederRequest::RevokeClient { .. } => Some(WriteKind::Revoke),
        FeederRequest::Reboot => Some(WriteKind::Reboot),
        FeederRequest::CancelUpdate => Some(WriteKind::Update),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::feeder::SettingsChanges;
    use crate::route::pins::PinnedWrite;

    #[test]
    fn a_wifi_change_pins_its_kind_alone_and_a_read_is_no_write() {
        let wifi = FeederRequest::PutWifi {
            base: 1,
            ssid: "home".into(),
            passphrase: "a secret phrase".into(),
        };
        let kind = write_kind(&wifi).expect("a Wi-Fi change is a write");
        let pinned = serde_json::to_string(&PinnedWrite::Other { kind }).unwrap();
        assert!(!pinned.contains("secret"), "{pinned}");

        let settings = FeederRequest::PutSettings {
            base: 3,
            changes: SettingsChanges {
                volume: Some(40),
                ..SettingsChanges::default()
            },
        };
        assert_eq!(write_kind(&settings), Some(WriteKind::Settings));
        assert_eq!(write_kind(&FeederRequest::GetPlans), None);
        assert_eq!(write_kind(&FeederRequest::GetCalibration), None);
        assert_eq!(
            write_kind(&FeederRequest::SkipMeal { slot: 2, due_ms: 0 }),
            Some(WriteKind::Skip)
        );
    }
}
