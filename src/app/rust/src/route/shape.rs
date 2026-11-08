//! What each request from Dart is to the router: a read, which goes wherever
//! it can, or a write, which carries a key and is pinned before it goes. A
//! write a server route carries has its body here, for its one send; any
//! other only its kind. The pin keeps the kind alone ([`PinnedWrite`]), so no
//! secret (a Wi-Fi passphrase) is ever written down, and nothing is ever sent
//! again from a pin.

use jiff::Timestamp;
use wellfed_client::call::Clip;
use wellfed_model::{Calibration, FeederSettingsPatch, Op, PLANS, Plan};

use crate::api::feeder::{FeederRequest, NotAsked};
use crate::api::router::WriteKind;
use crate::requests::{edited_plans, meal_due, portion_mg, settings_patch};
use crate::route::pins::PinnedWrite;

/// What a write other than a feed sends: the body of its one send, in
/// memory only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SentWrite {
    Cancel {
        feed: u32,
    },
    Settings {
        base: u32,
        changes: FeederSettingsPatch,
    },
    Lock {
        locked: bool,
    },
    Desiccant,
    /// A skip of one of today's meals, named by its instant.
    Skip {
        slot: u8,
        due: Timestamp,
    },
    Unskip {
        slot: u8,
        due: Timestamp,
    },
    Plans {
        base: u32,
        plans: [Option<Plan>; PLANS],
    },
    Calibration {
        base: u32,
        calibration: Calibration,
    },
    /// A meal call upload; its clip is the write's stream.
    CallUpload,
    CallDelete,
    CallPlay,
    /// A write no server route carries, sent on the phone's own session.
    Other {
        kind: WriteKind,
    },
}

impl SentWrite {
    pub(crate) const fn kind(&self) -> WriteKind {
        match self {
            Self::Cancel { .. } => WriteKind::Cancel,
            Self::Settings { .. } => WriteKind::Settings,
            Self::Lock { .. } => WriteKind::Lock,
            Self::Desiccant => WriteKind::Desiccant,
            Self::Skip { .. } | Self::Unskip { .. } => WriteKind::Skip,
            Self::Plans { .. } => WriteKind::Plans,
            Self::Calibration { .. } => WriteKind::Calibration,
            Self::CallUpload | Self::CallDelete | Self::CallPlay => WriteKind::Call,
            Self::Other { kind } => *kind,
        }
    }

    /// What its pin keeps of it.
    pub(crate) const fn pinned(&self) -> PinnedWrite {
        PinnedWrite::Other { kind: self.kind() }
    }
}

pub(crate) enum Shape {
    Read,
    /// A write, and for a meal call upload the stream the server takes.
    Write {
        write: SentWrite,
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

const fn write(write: SentWrite) -> Shape {
    Shape::Write {
        write,
        stream: None,
    }
}

const fn other(kind: WriteKind) -> Shape {
    write(SentWrite::Other { kind })
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
        FeederRequest::PutSettings { base, changes } => write(SentWrite::Settings {
            base: *base,
            changes: settings_patch(*changes),
        }),
        FeederRequest::SetLock { locked } => write(SentWrite::Lock { locked: *locked }),
        FeederRequest::ResetDesiccant => write(SentWrite::Desiccant),
        FeederRequest::SkipMeal { slot, due_ms } => match meal_due(*due_ms) {
            Some(due) => write(SentWrite::Skip { slot: *slot, due }),
            None => Shape::NotAsked(NotAsked::EmptySlot),
        },
        FeederRequest::UnskipMeal { slot, due_ms } => match meal_due(*due_ms) {
            Some(due) => write(SentWrite::Unskip { slot: *slot, due }),
            None => Shape::NotAsked(NotAsked::EmptySlot),
        },
        FeederRequest::EditPlans { plans, edit } => match edited_plans(plans, edit) {
            Ok((base, _, table)) => write(SentWrite::Plans {
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
            Ok(portion_mg) => write(SentWrite::Calibration {
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
                write: SentWrite::CallUpload,
                stream: Some(clip.stream().to_vec()),
            },
            Err(_) => Shape::NotAsked(NotAsked::BadRecording),
        },
        FeederRequest::DeleteCall => write(SentWrite::CallDelete),
        FeederRequest::PlayCall => write(SentWrite::CallPlay),
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
            SentWrite::Other {
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
            write: SentWrite::Settings { base, changes },
            ..
        } = shape(&settings)
        else {
            panic!("a settings change keeps its body");
        };
        assert_eq!((base, changes.volume), (3, Some(40)));
        assert!(matches!(shape(&FeederRequest::GetPlans), Shape::Read));
        assert!(matches!(shape(&FeederRequest::GetCalibration), Shape::Read));
    }

    /// A skip and its undo are writes the server carries, under their own
    /// kind, naming the meal by the instant the card gave; an instant that
    /// is no whole second names no meal and goes nowhere.
    #[test]
    fn a_skip_pins_its_meal_and_an_instant_off_the_second_is_refused() {
        let due_ms = 1_790_006_400_000;
        let due = jiff::Timestamp::from_millisecond(due_ms).unwrap();
        let Shape::Write { write, stream } = shape(&FeederRequest::SkipMeal { slot: 2, due_ms })
        else {
            panic!("a skip is a write");
        };
        assert_eq!(write, SentWrite::Skip { slot: 2, due });
        assert_eq!(write.kind(), WriteKind::Skip);
        assert!(stream.is_none());
        assert!(matches!(
            shape(&FeederRequest::UnskipMeal { slot: 2, due_ms }),
            Shape::Write {
                write: SentWrite::Unskip { slot: 2, .. },
                ..
            }
        ));
        assert!(matches!(
            shape(&FeederRequest::SkipMeal {
                slot: 2,
                due_ms: due_ms + 1
            }),
            Shape::NotAsked(NotAsked::EmptySlot)
        ));
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
                write: SentWrite::CallPlay,
                stream: None
            }
        ));
    }
}
