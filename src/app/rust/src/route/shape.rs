//! What each request from Dart is to the router: a read, which goes wherever
//! it can, or a write, which carries a key and is pinned before it goes. A
//! write a server route carries keeps its body in the pin, for the screens;
//! any other keeps only its kind, so no secret (a Wi-Fi passphrase) is ever
//! written down. Nothing is ever sent again from a pin.

use wellfed_client::call::Clip;
use wellfed_model::{Calibration, Op};

use crate::api::feeder::{FeederRequest, NotAsked};
use crate::api::router::WriteKind;
use crate::requests::{edited_plans, meal_due, portion_mg, settings_patch};
use crate::route::pins::PinnedWrite;

pub(crate) enum Shape {
    Read,
    /// A write, and for a meal call upload the stream the server takes.
    Write {
        write: PinnedWrite,
        stream: Option<Vec<u8>>,
    },
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

const fn write(write: PinnedWrite) -> Shape {
    Shape::Write {
        write,
        stream: None,
    }
}

const fn other(kind: WriteKind) -> Shape {
    write(PinnedWrite::Other { kind })
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
        FeederRequest::PutSettings { base, changes } => write(PinnedWrite::Settings {
            base: *base,
            changes: settings_patch(*changes),
        }),
        FeederRequest::SetLock { locked } => write(PinnedWrite::Lock { locked: *locked }),
        FeederRequest::ResetDesiccant => write(PinnedWrite::Desiccant),
        FeederRequest::SkipMeal { slot, due_ms } => match meal_due(*due_ms) {
            Some(due) => write(PinnedWrite::Skip { slot: *slot, due }),
            None => Shape::NotAsked(NotAsked::EmptySlot),
        },
        FeederRequest::UnskipMeal { slot, due_ms } => match meal_due(*due_ms) {
            Some(due) => write(PinnedWrite::Unskip { slot: *slot, due }),
            None => Shape::NotAsked(NotAsked::EmptySlot),
        },
        FeederRequest::EditPlans { plans, edit } => match edited_plans(plans, edit) {
            Ok((base, _, table)) => write(PinnedWrite::Plans {
                base: base.0,
                plans: table,
            }),
            Err(reason) => Shape::NotAsked(reason),
        },
        FeederRequest::PutCalibration {
            base,
            portion,
            bowl,
        } => match portion_mg(portion) {
            Ok(portion_mg) => write(PinnedWrite::Calibration {
                base: *base,
                calibration: Calibration {
                    portion_mg,
                    bowl: *bowl,
                },
            }),
            Err(reason) => Shape::NotAsked(reason),
        },
        FeederRequest::UploadCall { wav } => match Clip::from_wav(wav) {
            Ok(clip) => Shape::Write {
                write: PinnedWrite::CallUpload,
                stream: Some(clip.stream().to_vec()),
            },
            Err(_) => Shape::NotAsked(NotAsked::BadRecording),
        },
        FeederRequest::DeleteCall => write(PinnedWrite::CallDelete),
        FeederRequest::PlayCall => write(PinnedWrite::CallPlay),
        FeederRequest::PutWifi { .. } | FeederRequest::ForgetWifi { .. } => other(WriteKind::Wifi),
        FeederRequest::Play { .. } => other(WriteKind::Play),
        FeederRequest::SyncClock | FeederRequest::PutTime { .. } => other(WriteKind::Clock),
        FeederRequest::CreateInvite { .. } => other(WriteKind::Invite),
        FeederRequest::RevokeClient { .. } => other(WriteKind::Revoke),
        FeederRequest::Reboot => other(WriteKind::Reboot),
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
        let Shape::Write { write, stream } = shape(&wifi) else {
            panic!("a Wi-Fi change is a write");
        };
        assert_eq!(
            write,
            PinnedWrite::Other {
                kind: WriteKind::Wifi
            }
        );
        assert!(stream.is_none());

        let settings = FeederRequest::PutSettings {
            base: 3,
            changes: SettingsChanges {
                volume: Some(40),
                ..SettingsChanges::default()
            },
        };
        let Shape::Write {
            write: PinnedWrite::Settings { base, changes },
            ..
        } = shape(&settings)
        else {
            panic!("a settings change keeps its body");
        };
        assert_eq!((base, changes.volume), (3, Some(40)));
        assert!(matches!(shape(&FeederRequest::GetPlans), Shape::Read));
        assert!(matches!(shape(&FeederRequest::GetCalibration), Shape::Read));
    }

    #[test]
    fn a_recording_that_is_no_clip_is_refused_before_it_goes() {
        let upload = FeederRequest::UploadCall {
            wav: b"not a wav".to_vec(),
        };
        assert!(matches!(
            shape(&upload),
            Shape::NotAsked(NotAsked::BadRecording)
        ));
        assert!(matches!(
            shape(&FeederRequest::PlayCall),
            Shape::Write {
                write: PinnedWrite::CallPlay,
                stream: None
            }
        ));
    }
}
