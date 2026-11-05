//! The feeder's module of the device API ([`FeederOp`]): settings, the
//! lock and tunes, feeds and their traces, the calibration, the plans and
//! the meal call. The shared request path (`request`) checks the id, the
//! role and the retry, then routes an [`Action::Feeder`] here, or refuses it
//! `Unavailable` when the device has no feeder module
//! ([`crate::Device::feeder`]). What the feeder finishes later waits as a
//! [`FeederWait`], polled with every other write (`pending`), and its
//! answer goes out from [`Server::seal_feeder`].

use super::memory::Remembered;
use super::request::{Action, Reply};
use super::{Server, Waiting};
use crate::mailbox::{
    Calibration, CalibrationResult, CallChange, CallPlay, CallResult, LockResult, MealReminder,
    MealSkip, PLANS, Plan, PlansResult, PlayResult, SettingsFields, SettingsResult, Table,
    Weekdays, Window,
};
use crate::{FeederDevice, Reason};
use wellfed_core::tone::Tune as CoreTune;
use wellfed_proto::api::{
    self as proto, Bowl, CALL_BYTES_MAX, CALL_CHUNK_MAX, CallClip, ClientMessage, DeviceMessage,
    EventBody, FeederOp, FeederOutcome, Op, Outcome, Readiness, Refusal, RequestId, Source, Tune,
};

/// A feeder request's answer, compact as [`Reply`] is: the views are read
/// only when the answer is sealed ([`Server::seal_feeder`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FeederReply {
    Settings(proto::Settings),
    Feeding(u32),
    /// A trace's page.
    FeedTrace {
        feed: u32,
        offset: u16,
    },
    Calibration,
    Plans,
    /// The meal call.
    Call,
}

impl FeederReply {
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Settings(_) => "settings",
            Self::Feeding(_) => "feeding",
            Self::FeedTrace { .. } => "feed-trace",
            Self::Calibration => "calibration",
            Self::Plans => "plans",
            Self::Call => "call",
        }
    }
}

/// A feeder write the device is still completing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FeederWait {
    Settings,
    /// The digest of the calibration asked for: the write succeeded if the
    /// store holds it once nothing is pending.
    Calibration {
        digest: u32,
    },
    /// The revision a client's table written over `base` produces; a
    /// `Plans` event once the writer reports it saved.
    Plans {
        revision: u32,
    },
    /// A skip of slot `slot`'s meal due at `due_s`, or its undo: done once
    /// the slot's mark covers the meal (a skip) or no longer does (an
    /// undo), and recorded then.
    Skip {
        slot: u8,
        due_s: u32,
        skip: MealSkip,
    },
    /// A meal call upload's start, or its delete: done once the store
    /// waits for that upload's first chunk, or holds no clip.
    Call {
        change: CallChange,
    },
    Feed {
        feed: u32,
    },
    /// The feed it waited for ended refused, before anything moved.
    FeedRefused(Refusal),
}

/// What a feeder write's id keeps for a retry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FeederMemory {
    Feeding {
        feed: u32,
    },
    /// A feed queued under `feed` and not yet reserved: a retry waits for
    /// its reservation as the first request did, rather than queueing a
    /// second feed. Remembered from the moment the feed is queued, so a
    /// client whose session ended before the answer still gets it.
    Queued {
        feed: u32,
    },
}

/// What a feeder request asks, owned, as [`Action`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FeederAction {
    GetSettings,
    PutSettings {
        base: u32,
        fields: SettingsFields,
    },
    SetLock {
        locked: bool,
    },
    Play {
        tune: CoreTune,
    },
    Feed {
        portions: u8,
    },
    CancelFeed {
        feed: u32,
    },
    FeedTrace {
        feed: u32,
        offset: u16,
    },
    GetCalibration,
    PutCalibration {
        base: u32,
        calibration: Calibration,
        bowl: Bowl,
    },
    GetPlans,
    /// The table stays in the plaintext buffer, read again when the op
    /// runs.
    PutPlans {
        base: u32,
    },
    ResetDesiccant,
    /// A skip of slot `slot`'s meal due at `due_s`, or its undo.
    SkipMeal {
        slot: u8,
        due_s: u32,
        skip: MealSkip,
    },
    GetCall,
    StartCall {
        bytes: u32,
        crc: u32,
    },
    /// The bytes stay in the plaintext buffer, read again when the op runs.
    CallChunk {
        offset: u32,
    },
    DeleteCall,
    PlayCall,
}

/// A feeder op as the server runs it: [`Action::Invalid`] for one whose
/// fields fail the stores' checks before anything runs.
pub(super) fn action(op: FeederOp<'_>) -> Action {
    let action = match op {
        FeederOp::GetSettings => FeederAction::GetSettings,
        FeederOp::PutSettings { base, changes } => {
            let fields = SettingsFields {
                lights: changes.lights,
                sound: changes.sound,
                volume: changes.volume,
                lights_window: changes.lights_window.map(window),
                sound_window: changes.sound_window.map(window),
                desiccant_days: changes.desiccant_days,
                desiccant_reset: None,
                meal_reminder: changes.meal_reminder.map(reminder),
            };
            if !fields.valid() {
                return Action::Invalid;
            }
            FeederAction::PutSettings { base, fields }
        }
        FeederOp::SetLock { locked } => FeederAction::SetLock { locked },
        FeederOp::Play { tune } => FeederAction::Play {
            tune: core_tune(tune),
        },
        FeederOp::Feed { portions } => FeederAction::Feed { portions },
        FeederOp::CancelFeed { feed } => FeederAction::CancelFeed { feed },
        FeederOp::FeedTrace { feed, offset } => FeederAction::FeedTrace { feed, offset },
        FeederOp::GetCalibration => FeederAction::GetCalibration,
        FeederOp::PutCalibration { base, calibration } => {
            let bowl = calibration.bowl;
            let calibration = Calibration {
                portion_mg: calibration.portion_mg,
            };
            if !calibration.valid() {
                return Action::Invalid;
            }
            FeederAction::PutCalibration {
                base,
                calibration,
                bowl,
            }
        }
        FeederOp::GetPlans => FeederAction::GetPlans,
        // Checked when it runs ([`table`]), which names a plan's portions
        // in its refusal.
        FeederOp::PutPlans { base, .. } => FeederAction::PutPlans { base },
        FeederOp::ResetDesiccant => FeederAction::ResetDesiccant,
        FeederOp::GetCall => FeederAction::GetCall,
        FeederOp::StartCall { bytes, crc } => {
            if !(1..=CALL_BYTES_MAX).contains(&bytes) {
                return Action::Invalid;
            }
            FeederAction::StartCall { bytes, crc }
        }
        FeederOp::CallChunk { offset, data } => {
            if data.is_empty() || data.len() > CALL_CHUNK_MAX {
                return Action::Invalid;
            }
            FeederAction::CallChunk { offset }
        }
        FeederOp::DeleteCall => FeederAction::DeleteCall,
        FeederOp::PlayCall => FeederAction::PlayCall,
        FeederOp::SkipMeal { slot, due_s } => FeederAction::SkipMeal {
            slot,
            due_s,
            skip: MealSkip::Skip,
        },
        FeederOp::UnskipMeal { slot, due_s } => FeederAction::SkipMeal {
            slot,
            due_s,
            skip: MealSkip::Unskip,
        },
    };
    Action::Feeder(action)
}

impl FeederAction {
    /// The meal call's chunk op, whose answers the records leave out.
    pub(super) const CALL_CHUNK: &'static str = "call-chunk";

    /// Whether the request changes something, so its id is remembered.
    pub(super) const fn writes(self) -> bool {
        !matches!(
            self,
            Self::GetSettings
                | Self::FeedTrace { .. }
                | Self::GetCalibration
                | Self::GetPlans
                | Self::GetCall
                | Self::CallChunk { .. }
        )
    }

    /// Whether only an owner may run it: the meal call is the owner's
    /// voice, or the clip they chose. A member also sets the portion
    /// weight; the server's key does not (`request::allowed`).
    pub(super) const fn owners_only(self) -> bool {
        matches!(
            self,
            Self::StartCall { .. } | Self::CallChunk { .. } | Self::DeleteCall
        )
    }

    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::GetSettings => "get-settings",
            Self::PutSettings { .. } => "put-settings",
            Self::SetLock { .. } => "set-lock",
            Self::Play { .. } => "play",
            Self::Feed { .. } => "feed",
            Self::CancelFeed { .. } => "cancel-feed",
            Self::FeedTrace { .. } => "feed-trace",
            Self::GetCalibration => "get-calibration",
            Self::PutCalibration { .. } => "put-calibration",
            Self::GetPlans => "get-plans",
            Self::PutPlans { .. } => "put-plans",
            Self::ResetDesiccant => "reset-desiccant",
            Self::GetCall => "get-call",
            Self::StartCall { .. } => "start-call",
            Self::CallChunk { .. } => Self::CALL_CHUNK,
            Self::DeleteCall => "delete-call",
            Self::PlayCall => "play-call",
            Self::SkipMeal {
                skip: MealSkip::Skip,
                ..
            } => "skip-meal",
            Self::SkipMeal {
                skip: MealSkip::Unskip,
                ..
            } => "unskip-meal",
        }
    }
}

impl Server {
    /// Answer or queue one feeder request, which the shared path has
    /// already let through: `None` when the answer waits on the device.
    #[expect(clippy::too_many_arguments, reason = "one request's whole context")]
    pub(super) fn feeder_request<F: FeederDevice>(
        &mut self,
        index: usize,
        id: RequestId,
        action: FeederAction,
        memory: usize,
        by: Source,
        now_ms: u32,
        device: &mut F,
    ) -> Option<Reply> {
        let whole = Action::Feeder(action);
        let outcome = match action {
            FeederAction::GetSettings => match device.settings(SettingsFields::default()) {
                SettingsResult::Current { view, .. } => {
                    Reply::Feeder(FeederReply::Settings(proto::Settings {
                        generation: view.generation,
                        lights: view.lights,
                        sound: view.sound,
                        volume: view.volume,
                        lights_window: wire_window(view.lights_window),
                        sound_window: wire_window(view.sound_window),
                        desiccant_days: view.desiccant_days,
                        desiccant_reset: view.desiccant_reset,
                        meal_reminder: wire_reminder(view.meal_reminder),
                        writable: view.writable,
                    }))
                }
                SettingsResult::Busy => Reply::Refused(Refusal::Busy),
                SettingsResult::ReadOnly => Reply::Refused(Refusal::NotWritable),
            },
            FeederAction::PutSettings { base, fields } => {
                match put_settings(base, fields, device) {
                    Ok(()) => {
                        return self.wait(index, id, whole, Waiting::Feeder(FeederWait::Settings));
                    }
                    Err(refusal) => Reply::Refused(refusal),
                }
            }
            FeederAction::ResetDesiccant => match reset_desiccant(now_ms, device) {
                Ok(()) => {
                    return self.wait(index, id, whole, Waiting::Feeder(FeederWait::Settings));
                }
                Err(refusal) => Reply::Refused(refusal),
            },
            FeederAction::SetLock { locked } => match device.lock(locked) {
                LockResult::Current { .. } => {
                    device.record(now_ms, EventBody::Lock { locked, by });
                    Reply::Done(None)
                }
                LockResult::Busy => Reply::Refused(Refusal::Busy),
                LockResult::Unserved => Reply::Refused(Refusal::Unavailable),
            },
            FeederAction::Play { tune } => match device.play(tune) {
                PlayResult::Started => Reply::Done(None),
                PlayResult::Muted => Reply::Refused(Refusal::Muted),
                PlayResult::Busy => Reply::Refused(Refusal::Busy),
            },
            FeederAction::PlayCall => match device.play_call() {
                CallPlay::Started => Reply::Done(None),
                CallPlay::Muted => Reply::Refused(Refusal::Muted),
                CallPlay::Busy => Reply::Refused(Refusal::Busy),
                CallPlay::NoClip => Reply::Refused(Refusal::NotFound),
                CallPlay::Unserved => Reply::Refused(Refusal::Unavailable),
            },
            FeederAction::GetCalibration => match device.calibration(None) {
                CalibrationResult::Current { .. } => Reply::Feeder(FeederReply::Calibration),
                CalibrationResult::Busy => Reply::Refused(Refusal::Busy),
                CalibrationResult::ReadOnly | CalibrationResult::Unserved => {
                    Reply::Refused(Refusal::Unavailable)
                }
            },
            // A build runs the one bowl mode it was verified for: another
            // would turn the rotor a way no bench run has checked.
            FeederAction::PutCalibration { bowl, .. } if bowl != device.bowl() => {
                Reply::Refused(Refusal::Unverified)
            }
            FeederAction::PutCalibration {
                base, calibration, ..
            } => match put_calibration(base, calibration, device) {
                Ok(digest) => {
                    return self.wait(
                        index,
                        id,
                        whole,
                        Waiting::Feeder(FeederWait::Calibration { digest }),
                    );
                }
                Err(refusal) => Reply::Refused(refusal),
            },
            FeederAction::GetPlans => match device.plans(None) {
                PlansResult::Current { .. } => Reply::Feeder(FeederReply::Plans),
                PlansResult::Busy => Reply::Refused(Refusal::Busy),
                PlansResult::ReadOnly | PlansResult::Unserved => {
                    Reply::Refused(Refusal::Unavailable)
                }
            },
            FeederAction::PutPlans { base } => match self.put_plans(base, device) {
                Ok(revision) => {
                    return self.wait(
                        index,
                        id,
                        whole,
                        Waiting::Feeder(FeederWait::Plans { revision }),
                    );
                }
                Err(refusal) => Reply::Refused(refusal),
            },
            FeederAction::SkipMeal { slot, due_s, skip } => {
                match device.skip_meal(slot, due_s, skip, now_ms) {
                    Ok(()) => {
                        let wait = FeederWait::Skip { slot, due_s, skip };
                        return self.wait(index, id, whole, Waiting::Feeder(wait));
                    }
                    Err(refusal) => Reply::Refused(refusal),
                }
            }
            FeederAction::GetCall => match device.call(None) {
                CallResult::Current { .. } => Reply::Feeder(FeederReply::Call),
                CallResult::Busy => Reply::Refused(Refusal::Busy),
                CallResult::ReadOnly | CallResult::Unserved => Reply::Refused(Refusal::Unavailable),
            },
            FeederAction::StartCall { bytes, crc } => {
                let change = CallChange::Start { bytes, crc };
                match put_call(change, device) {
                    Ok(()) => {
                        return self.wait(
                            index,
                            id,
                            whole,
                            Waiting::Feeder(FeederWait::Call { change }),
                        );
                    }
                    Err(refusal) => Reply::Refused(refusal),
                }
            }
            FeederAction::DeleteCall => match put_call(CallChange::Delete, device) {
                Ok(()) => {
                    let change = CallChange::Delete;
                    return self.wait(
                        index,
                        id,
                        whole,
                        Waiting::Feeder(FeederWait::Call { change }),
                    );
                }
                Err(refusal) => Reply::Refused(refusal),
            },
            FeederAction::CallChunk { offset } => match self.call_chunk(offset, device) {
                Ok(()) => Reply::Feeder(FeederReply::Call),
                Err(refusal) => Reply::Refused(refusal),
            },
            // Named before anything else is checked, so the refusal says
            // what to change rather than when to try again.
            FeederAction::Feed { portions } if !(1..=proto::PORTIONS_MAX).contains(&portions) => {
                Reply::Refused(Refusal::Portions { asked: portions })
            }
            // A restart is due: a feed now would be cut by it.
            FeederAction::Feed { .. } if self.reboot_at.is_some() => Reply::Refused(Refusal::Busy),
            FeederAction::Feed { portions } => match device.readiness() {
                Readiness::Blocked(blocked) => Reply::Refused(blocked.refusal()),
                Readiness::Ready => match device.feed(portions, by) {
                    Ok(feed) => {
                        self.memories[memory]
                            .remember(id, Remembered::Feeder(FeederMemory::Queued { feed }));
                        return self.wait(
                            index,
                            id,
                            whole,
                            Waiting::Feeder(FeederWait::Feed { feed }),
                        );
                    }
                    Err(refusal) => Reply::Refused(refusal),
                },
            },
            FeederAction::CancelFeed { feed } => {
                if device.cancel_feed(feed) {
                    Reply::Done(None)
                } else {
                    Reply::Refused(Refusal::NotFound)
                }
            }
            FeederAction::FeedTrace { feed, offset } => {
                return Some(Reply::Feeder(FeederReply::FeedTrace { feed, offset }));
            }
        };
        Some(self.remember(memory, id, whole, outcome))
    }

    /// The meal call's chunk in `plain`, handed to the device. Out of line,
    /// as for an update's.
    #[inline(never)]
    fn call_chunk<F: FeederDevice>(&self, offset: u32, device: &mut F) -> Result<(), Refusal> {
        let message: ClientMessage<'_> =
            wellfed_proto::decode(&self.plain[..self.plain_len]).map_err(|_| Refusal::Invalid)?;
        match message {
            ClientMessage::Request {
                op: Op::Feeder(FeederOp::CallChunk { offset: at, data }),
                ..
            } if at == offset => device.call_chunk(offset, data),
            _ => Err(Refusal::Invalid),
        }
    }

    /// Queue the table in `plain` if the plans are still at `base`; the
    /// revision it produces once saved. Out of line, as for an update: the
    /// table is read again from the plaintext rather than carried in the
    /// action.
    #[inline(never)]
    fn put_plans<F: FeederDevice>(&self, base: u32, device: &mut F) -> Result<u32, Refusal> {
        let message: ClientMessage<'_> =
            wellfed_proto::decode(&self.plain[..self.plain_len]).map_err(|_| Refusal::Invalid)?;
        let ClientMessage::Request {
            op: Op::Feeder(FeederOp::PutPlans { plans, .. }),
            ..
        } = message
        else {
            return Err(Refusal::Invalid);
        };
        let table = table(&plans)?;

        let view = match device.plans(None) {
            PlansResult::Current { pending: true, .. } | PlansResult::Busy => {
                return Err(Refusal::Busy);
            }
            PlansResult::ReadOnly => return Err(Refusal::NotWritable),
            PlansResult::Unserved => return Err(Refusal::Unavailable),
            PlansResult::Current { view, .. } => view,
        };
        if view.revision != base {
            return Err(Refusal::Stale {
                current: view.revision,
            });
        }

        match device.plans(Some(table)) {
            PlansResult::Current { pending: true, .. } => Ok(base.wrapping_add(1)),
            PlansResult::Current { .. } | PlansResult::Busy => Err(Refusal::Busy),
            PlansResult::ReadOnly => Err(Refusal::NotWritable),
            PlansResult::Unserved => Err(Refusal::Unavailable),
        }
    }
}

fn put_settings<F: FeederDevice>(
    base: u32,
    fields: SettingsFields,
    device: &mut F,
) -> Result<(), Refusal> {
    match device.settings(SettingsFields::default()) {
        SettingsResult::Current { pending: true, .. } | SettingsResult::Busy => Err(Refusal::Busy),
        SettingsResult::ReadOnly => Err(Refusal::NotWritable),
        SettingsResult::Current { view, .. } if view.generation != base => Err(Refusal::Stale {
            current: view.generation,
        }),
        SettingsResult::Current { .. } => match device.settings(fields) {
            SettingsResult::Current { pending: true, .. } => Ok(()),
            SettingsResult::Current { .. } | SettingsResult::Busy => Err(Refusal::Busy),
            SettingsResult::ReadOnly => Err(Refusal::NotWritable),
        },
    }
}

/// Queue a meal call change for the writer. A new upload or a delete
/// replaces whatever the store holds, so neither names a `base`.
fn put_call<F: FeederDevice>(change: CallChange, device: &mut F) -> Result<(), Refusal> {
    match device.call(None) {
        CallResult::Current { pending: true, .. } | CallResult::Busy => return Err(Refusal::Busy),
        CallResult::ReadOnly => return Err(Refusal::NotWritable),
        CallResult::Unserved => return Err(Refusal::Unavailable),
        CallResult::Current { .. } => {}
    }
    match device.call(Some(change)) {
        CallResult::Current { pending: true, .. } => Ok(()),
        CallResult::Current { .. } | CallResult::Busy => Err(Refusal::Busy),
        CallResult::ReadOnly => Err(Refusal::NotWritable),
        CallResult::Unserved => Err(Refusal::Unavailable),
    }
}

/// Queue a desiccant replacement dated today. The day comes from the clock
/// alone: with no valid clock the request is refused rather than dated by
/// a guess. No `base`: a replacement is a fact about today, whatever the
/// settings hold, and the settings event it records names their new
/// generation.
fn reset_desiccant<F: FeederDevice>(now_ms: u32, device: &mut F) -> Result<(), Refusal> {
    let today = device.today(now_ms).ok_or(Refusal::NoClock)?;

    match device.settings(SettingsFields::default()) {
        SettingsResult::Current { pending: true, .. } | SettingsResult::Busy => {
            return Err(Refusal::Busy);
        }
        SettingsResult::ReadOnly => return Err(Refusal::NotWritable),
        SettingsResult::Current { .. } => {}
    }
    let fields = SettingsFields {
        desiccant_reset: Some(today),
        ..SettingsFields::default()
    };
    match device.settings(fields) {
        SettingsResult::Current { pending: true, .. } => Ok(()),
        SettingsResult::Current { .. } | SettingsResult::Busy => Err(Refusal::Busy),
        SettingsResult::ReadOnly => Err(Refusal::NotWritable),
    }
}

/// Queue `calibration` if the store is still at `base`; the digest of what
/// was asked for.
fn put_calibration<F: FeederDevice>(
    base: u32,
    calibration: Calibration,
    device: &mut F,
) -> Result<u32, Refusal> {
    let view = match device.calibration(None) {
        CalibrationResult::Current { pending: true, .. } | CalibrationResult::Busy => {
            return Err(Refusal::Busy);
        }
        CalibrationResult::ReadOnly => return Err(Refusal::NotWritable),
        CalibrationResult::Unserved => return Err(Refusal::Unavailable),
        CalibrationResult::Current { view, .. } => view,
    };
    if view.generation != base {
        return Err(Refusal::Stale {
            current: view.generation,
        });
    }

    match device.calibration(Some(calibration)) {
        CalibrationResult::Current { pending: true, .. } => Ok(calibration.digest()),
        CalibrationResult::Current { .. } | CalibrationResult::Busy => Err(Refusal::Busy),
        CalibrationResult::ReadOnly => Err(Refusal::NotWritable),
        CalibrationResult::Unserved => Err(Refusal::Unavailable),
    }
}

const fn core_tune(tune: Tune) -> CoreTune {
    match tune {
        Tune::Chime => CoreTune::Chime,
        Tune::Meal => CoreTune::Meal,
        Tune::Coin => CoreTune::Coin,
        Tune::Alert => CoreTune::Alert,
        Tune::Test => CoreTune::Test,
        Tune::Twinkle => CoreTune::Twinkle,
    }
}

/// A window as the settings mailbox carries it.
fn window(window: proto::Window) -> Window {
    match window {
        proto::Window::Always => Window::Always,
        proto::Window::Between { start, end } => Window::Between { start, end },
    }
}

/// A window as the wire carries it.
fn wire_window(window: Window) -> proto::Window {
    match window {
        Window::Always => proto::Window::Always,
        Window::Between { start, end } => proto::Window::Between { start, end },
    }
}

fn reminder(reminder: proto::MealReminder) -> MealReminder {
    match reminder {
        proto::MealReminder::Off => MealReminder::Off,
        proto::MealReminder::Before { minutes } => MealReminder::Before { minutes },
    }
}

/// A meal reminder as the wire carries it.
fn wire_reminder(reminder: MealReminder) -> proto::MealReminder {
    match reminder {
        MealReminder::Off => proto::MealReminder::Off,
        MealReminder::Before { minutes } => proto::MealReminder::Before { minutes },
    }
}

const _: () = assert!(PLANS == proto::PLANS_MAX);
const _: () = assert!(wellfed_core::schedule::PORTIONS_MAX == proto::PORTIONS_MAX);
const _: () = assert!(
    wellfed_core::desiccant::PERIOD_MIN == proto::DESICCANT_DAYS_MIN
        && wellfed_core::desiccant::PERIOD_MAX == proto::DESICCANT_DAYS_MAX
        && wellfed_core::desiccant::PERIOD_DEFAULT == proto::DESICCANT_DAYS_DEFAULT
);
const _: () = assert!(
    wellfed_core::settings::REMINDER_MINUTES_MIN == proto::MEAL_REMINDER_MINUTES_MIN
        && wellfed_core::settings::REMINDER_MINUTES_MAX == proto::MEAL_REMINDER_MINUTES_MAX
);

/// The wire's plans as the store keeps them. A plan's portions outside
/// 1 to [`proto::PORTIONS_MAX`] are refused by name, as a feed's are; any
/// other field that fails its checks is `Invalid`.
fn table(plans: &[Option<proto::Plan>; PLANS]) -> Result<Table, Refusal> {
    let mut table: Table = [None; PLANS];
    for (slot, plan) in table.iter_mut().zip(plans) {
        *slot = match *plan {
            None => None,
            Some(plan) => {
                let plan = Plan {
                    minute: plan.minute,
                    portions: plan.portions,
                    days: Weekdays(plan.days),
                    enabled: plan.enabled,
                    call: plan.call,
                };
                if !(1..=proto::PORTIONS_MAX).contains(&plan.portions) {
                    return Err(Refusal::Portions {
                        asked: plan.portions,
                    });
                }
                if !plan.valid() {
                    return Err(Refusal::Invalid);
                }
                Some(plan)
            }
        };
    }
    Ok(table)
}

/// The store's plans as the wire carries them.
pub(super) fn wire_plans(table: &Table) -> [Option<proto::Plan>; PLANS] {
    table.map(|plan| {
        plan.map(|plan| proto::Plan {
            minute: plan.minute,
            portions: plan.portions,
            days: plan.days.0,
            enabled: plan.enabled,
            call: plan.call,
        })
    })
}

impl Server {
    /// Seal a feeder reply, read from the feeder's module when its answer
    /// goes out. Out of line, as `seal_answer` is: the wire message is as
    /// large as its largest variant.
    #[inline(never)]
    pub(super) fn seal_feeder<F: FeederDevice>(
        &mut self,
        index: usize,
        id: RequestId,
        reply: FeederReply,
        feeder: Option<&mut F>,
    ) -> Result<(), Reason> {
        let unavailable = Outcome::Refused(Refusal::Unavailable);
        let Some(feeder) = feeder else {
            let message = DeviceMessage::Response {
                id,
                result: unavailable,
            };
            return self.seal_and_send(index, &message);
        };
        let result = match reply {
            FeederReply::Settings(settings) => Outcome::Feeder(FeederOutcome::Settings(settings)),
            FeederReply::Feeding(feed) => Outcome::Feeder(FeederOutcome::Feeding { feed }),
            FeederReply::Calibration => match feeder.calibration(None) {
                CalibrationResult::Current { view, .. } => {
                    Outcome::Feeder(FeederOutcome::Calibration(proto::CalibrationView {
                        generation: view.generation,
                        writable: view.writable,
                        calibration: proto::Calibration {
                            portion_mg: view.calibration.portion_mg,
                            bowl: feeder.bowl(),
                        },
                        phase: feeder.phase(),
                    }))
                }
                _ => unavailable,
            },
            FeederReply::Plans => match feeder.plans(None) {
                PlansResult::Current { view, .. } => {
                    Outcome::Feeder(FeederOutcome::Plans(proto::Plans {
                        generation: view.revision,
                        writable: view.writable,
                        plans: wire_plans(&view.table),
                        skipped: core::array::from_fn(|slot| view.skipped(slot)),
                    }))
                }
                _ => unavailable,
            },
            FeederReply::Call => match feeder.call(None) {
                CallResult::Current { view, .. } => Outcome::Feeder(FeederOutcome::Call(view)),
                CallResult::Busy => Outcome::Refused(Refusal::Busy),
                CallResult::ReadOnly | CallResult::Unserved => unavailable,
            },
            FeederReply::FeedTrace { feed, offset } => match feeder.feed_trace(feed, offset) {
                Ok(page) => Outcome::Feeder(FeederOutcome::FeedTrace(page)),
                Err(refusal) => Outcome::Refused(refusal),
            },
        };
        self.seal_and_send(index, &DeviceMessage::Response { id, result })
    }
}

/// A pending feeder write's answer once the feeder has finished it; `None`
/// while it is still under way.
pub(super) fn poll<F: FeederDevice>(
    wait: FeederWait,
    now_ms: u32,
    by: Source,
    device: &mut F,
) -> Option<Reply> {
    Some(match wait {
        FeederWait::Settings => match device.settings(SettingsFields::default()) {
            SettingsResult::Current {
                view,
                pending: false,
            } => {
                let generation = view.generation;
                device.record(now_ms, EventBody::Settings { generation, by });
                Reply::Done(Some(generation))
            }
            _ => return None,
        },
        // No event: a calibration has its own generation.
        FeederWait::Calibration { digest } => match device.calibration(None) {
            CalibrationResult::Current {
                view,
                pending: false,
            } if view.calibration.digest() == digest => Reply::Done(Some(view.generation)),
            CalibrationResult::Current { pending: false, .. } => {
                Reply::Refused(Refusal::NotWritable)
            }
            CalibrationResult::Current { pending: true, .. } | CalibrationResult::Busy => {
                return None;
            }
            CalibrationResult::ReadOnly | CalibrationResult::Unserved => {
                Reply::Refused(Refusal::NotWritable)
            }
        },
        // The loop may write the plans right after the client's table
        // (a plan that runs once disabling itself), so the writer's
        // record of that write decides, not the table now.
        FeederWait::Plans { revision } => match device.plans(None) {
            PlansResult::Current {
                view,
                pending: false,
            } if view.replaced == revision => {
                let generation = revision;
                device.record(now_ms, EventBody::Plans { generation, by });
                Reply::Done(Some(generation))
            }
            PlansResult::Current { pending: false, .. } => Reply::Refused(Refusal::NotWritable),
            PlansResult::Current { pending: true, .. } | PlansResult::Busy => return None,
            PlansResult::ReadOnly | PlansResult::Unserved => Reply::Refused(Refusal::NotWritable),
        },
        // Only a skip puts a mark ahead of the clock, and the meal is still
        // to come, so the slot's mark tells whether the store holds it.
        FeederWait::Skip { slot, due_s, skip } => match device.plans(None) {
            PlansResult::Current {
                view,
                pending: false,
            } => {
                let covered = view
                    .mark(usize::from(slot))
                    .is_some_and(|mark| mark >= due_s);
                let (landed, body) = match skip {
                    MealSkip::Skip => (
                        covered,
                        EventBody::Skipped {
                            plan: slot,
                            due: due_s,
                            by,
                        },
                    ),
                    MealSkip::Unskip => (
                        !covered,
                        EventBody::Unskipped {
                            plan: slot,
                            due: due_s,
                            by,
                        },
                    ),
                };
                if landed {
                    device.record(now_ms, body);
                    Reply::Done(None)
                } else {
                    Reply::Refused(Refusal::NotWritable)
                }
            }
            PlansResult::Current { pending: true, .. } | PlansResult::Busy => return None,
            PlansResult::ReadOnly | PlansResult::Unserved => Reply::Refused(Refusal::NotWritable),
        },
        // A write the writer could not make leaves the store as it
        // was: refused, never `Done`.
        FeederWait::Call { change } => match device.call(None) {
            CallResult::Current {
                view,
                pending: false,
            } => {
                let landed = match (change, view.clip) {
                    (CallChange::Delete, CallClip::None) => true,
                    (
                        CallChange::Start { bytes, crc },
                        CallClip::Receiving {
                            bytes: held,
                            crc: held_crc,
                            ..
                        },
                    ) => held == bytes && held_crc == crc,
                    _ => false,
                };
                if landed {
                    Reply::Done(None)
                } else {
                    Reply::Refused(Refusal::NotWritable)
                }
            }
            CallResult::Current { pending: true, .. } | CallResult::Busy => return None,
            CallResult::ReadOnly | CallResult::Unserved => Reply::Refused(Refusal::NotWritable),
        },
        FeederWait::Feed { feed } => match device.feed_reserved(feed) {
            None => return None,
            Some(Ok(())) => Reply::Feeder(FeederReply::Feeding(feed)),
            Some(Err(refusal)) => Reply::Refused(refusal),
        },
        FeederWait::FeedRefused(refusal) => Reply::Refused(refusal),
    })
}
