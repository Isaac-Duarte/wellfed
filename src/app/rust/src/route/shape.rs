//! What each request from Dart is to the router: a read, which goes wherever
//! it can, or a write, which carries a key and is pinned when it may have
//! run. A write the server can carry again under its key keeps its body; any
//! other keeps only its kind, so no secret (a Wi-Fi passphrase) is ever
//! written down.

use wellfed_model::Op;

use crate::api::feeder::{FeederRequest, NotAsked};
use crate::api::router::WriteKind;
use crate::requests::{edited_plans, settings_patch};
use crate::route::pins::PinnedWrite;

pub(crate) enum Shape {
    Read,
    Write(PinnedWrite),
    /// Refused here, before anything was sent.
    NotAsked(NotAsked),
}

/// How a write goes on the phone's own session.
pub(crate) enum DirectCall {
    /// Through the request's own ops, as a session without a server runs it.
    Request(FeederRequest),
    /// One op, answered to the router.
    Op(Op),
}

const fn other(kind: WriteKind) -> Shape {
    Shape::Write(PinnedWrite::Other { kind })
}

pub(crate) fn shape(request: &FeederRequest) -> Shape {
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
        | FeederRequest::TailLog { .. } => Shape::Read,
        FeederRequest::PutSettings { base, changes } => Shape::Write(PinnedWrite::Settings {
            base: *base,
            changes: settings_patch(*changes),
        }),
        FeederRequest::SetLock { locked } => Shape::Write(PinnedWrite::Lock { locked: *locked }),
        FeederRequest::ResetDesiccant => Shape::Write(PinnedWrite::Desiccant),
        FeederRequest::EditPlans { plans, edit } => match edited_plans(plans, edit) {
            Ok((base, _, table)) => Shape::Write(PinnedWrite::Plans {
                base: base.0,
                plans: table,
            }),
            Err(reason) => Shape::NotAsked(reason),
        },
        FeederRequest::PutWifi { .. } | FeederRequest::ForgetWifi { .. } => other(WriteKind::Wifi),
        FeederRequest::Play { .. } => other(WriteKind::Play),
        FeederRequest::SyncClock | FeederRequest::PutTime { .. } => other(WriteKind::Clock),
        FeederRequest::CreateInvite { .. } => other(WriteKind::Invite),
        FeederRequest::RevokeClient { .. } => other(WriteKind::Revoke),
        FeederRequest::Reboot => other(WriteKind::Reboot),
        FeederRequest::PutCalibration { .. } => other(WriteKind::Calibration),
        FeederRequest::UploadCall { .. } | FeederRequest::DeleteCall | FeederRequest::PlayCall => {
            other(WriteKind::Call)
        }
        FeederRequest::CancelUpdate => other(WriteKind::Update),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::feeder::SettingsChanges;

    #[test]
    fn a_wifi_change_keeps_no_passphrase_and_a_settings_change_keeps_its_body() {
        let wifi = FeederRequest::PutWifi {
            base: 1,
            ssid: "home".into(),
            passphrase: "a secret phrase".into(),
        };
        let Shape::Write(write) = shape(&wifi) else {
            panic!("a Wi-Fi change is a write");
        };
        assert_eq!(
            write,
            PinnedWrite::Other {
                kind: WriteKind::Wifi
            }
        );

        let settings = FeederRequest::PutSettings {
            base: 3,
            changes: SettingsChanges {
                volume: Some(40),
                ..SettingsChanges::default()
            },
        };
        let Shape::Write(PinnedWrite::Settings { base, changes }) = shape(&settings) else {
            panic!("a settings change keeps its body");
        };
        assert_eq!((base, changes.volume), (3, Some(40)));
        assert!(matches!(shape(&FeederRequest::GetPlans), Shape::Read));
    }
}
