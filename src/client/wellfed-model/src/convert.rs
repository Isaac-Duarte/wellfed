//! The wire's values to this crate's, and back where a host sends or
//! stores the wire form, in one place.
//!
//! Every match here is exhaustive and every struct is destructured
//! without `..`, so a variant or field added to `wellfed-proto` does not
//! build until it has a host meaning.

use jiff::Timestamp;
use wellfed_proto::api as proto;
use wellfed_proto::device::DeviceKind;
use wellfed_proto::setup as proto_setup;

use crate::{Alert, AlertsView, OpenAlert, Pushed, UnknownEvent, UpdateCandidate};
use crate::{
    Answer, Bank0Blocked, Bank0Check, Bank1Page, Battery, Blocked, Bowl, Calibration,
    CalibrationView, CallClip, CallView, Client, ClientRole, CoreEvent, Counters, Counts,
    Desiccant, DeviceStatus, EnrollPath, Erases, Event, EventBody, FeedFault, FeedOutcome,
    FeedReason, FeederEvent, FeederSettings, FeederSettingsPatch, FeederStatus, Flash, Food,
    FoodLevel, Generation, Hello, Hex, Invite, Joined, LogLines, MealReminder, Memory,
    MissedReason, NtpResult, NtpServer, Op, OpenRefusal, Phase, Plan, Plans, Portions, Power,
    PowerStats, Readiness, Refusal, ResetCause, Resets, Sample, SampleValue, Settings,
    SkippedMeals, Source, Stats, StatsSection, Status, Supply, TaskStack, TimeSource, TimeSync,
    TimeView, Tune, UpdatePhase, UpdateRefusal, UpdateStop, UpdateView, Weekdays, WifiSource,
    Window, setup,
};
use crate::{
    CurrentUnit, FeedStats, Malformed, Trace, TraceEntry, TraceMotor, TracePage, TraceSummary,
};
use wellfed_proto::trace as proto_trace;

/// One log batch: its text is whole lines, each ending in `\n`.
pub fn log(seq: u32, dropped: u32, text: &str) -> LogLines {
    LogLines {
        seq,
        dropped,
        lines: text.lines().map(str::to_owned).collect(),
    }
}

/// One event of the history `epoch` counts, as a batch carries it: an
/// [`UnknownEvent`] when this build does not know its body.
pub fn raw_event(epoch: u32, raw: proto::RawEvent) -> Pushed {
    match raw.event() {
        Some(known) => Pushed::Event(event(epoch, known)),
        None => {
            let proto::RawEvent { seq, at, body } = raw;
            Pushed::Unknown(UnknownEvent {
                epoch,
                seq,
                at: at.map(seconds),
                raw: Hex(body),
            })
        }
    }
}

/// One event of the history `epoch` counts.
pub fn event(epoch: u32, event: proto::Event) -> Event {
    let proto::Event { seq, at, body } = event;
    Event {
        epoch,
        seq,
        at: at.map(seconds),
        body: body.into(),
    }
}

/// A unit-only enum an answer carries leniently, as this build reads it:
/// `unknown` for a variant a newer build added.
fn lenient<P: Copy, M: From<P>>(value: proto::Lenient<P>, unknown: M) -> M {
    value.known().map_or(unknown, M::from)
}

/// Each unit enum that alerts and event bodies carry leniently, read from
/// the wire with `Unknown` for a variant a newer build added, and written
/// back as the variant it names.
macro_rules! lenient_both_ways {
    ($($model:ty => $wire:ty),* $(,)?) => {$(
        impl From<proto::Lenient<$wire>> for $model {
            fn from(value: proto::Lenient<$wire>) -> Self {
                lenient(value, Self::Unknown)
            }
        }

        impl TryFrom<$model> for proto::Lenient<$wire> {
            type Error = NoWireForm;

            fn try_from(value: $model) -> Result<Self, NoWireForm> {
                <$wire>::try_from(value).map(proto::Lenient::Known)
            }
        }
    )*};
}

lenient_both_ways! {
    FeedOutcome => proto::FeedOutcome,
    FeedReason => proto::FeedReason,
    FoodLevel => proto::FoodLevel,
    Blocked => proto::Blocked,
    ClientRole => proto::Role,
    EnrollPath => proto::Path,
    UpdateStop => proto::UpdateStop,
    UpdateCandidate => proto::UpdateCandidate,
}

/// A host value with no wire form: one holding a variant a newer feeder
/// build added, which this build only names `Unknown`. Never so for one
/// decoded from the wire, where a body this build cannot read stays raw.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the value holds one this build cannot name, which has no wire form here")]
pub struct NoWireForm;

/// An event's time: the feeder's clock in Unix seconds.
fn seconds(at: u32) -> Timestamp {
    Timestamp::from_second(i64::from(at)).expect("a u32 of seconds is inside jiff's range")
}

impl From<proto::Status<'_>> for Status {
    fn from(status: proto::Status<'_>) -> Self {
        let proto::Status {
            build,
            bank,
            uptime_ms,
            boot_count,
            reset_flags,
            faults,
            rssi,
            wifi,
            link_losses,
            dns_losses,
            clock_ms,
            power,
            wifi_generation,
            last_event,
            sessions,
            alerts_generation,
            device,
            ext: _,
        } = status;
        Self {
            build: build.to_owned(),
            bank,
            uptime_ms,
            boot_count,
            reset_flags,
            faults,
            rssi,
            wifi: lenient(wifi, WifiSource::Unknown),
            link_losses,
            dns_losses,
            clock: clock_ms.and_then(|ms| {
                i64::try_from(ms)
                    .ok()
                    .and_then(|ms| Timestamp::from_millisecond(ms).ok())
            }),
            power: power.map(Power::from),
            wifi_generation: Generation(wifi_generation),
            last_event,
            sessions,
            alerts_generation: Generation(alerts_generation),
            device: device.into(),
        }
    }
}

impl From<proto::DeviceStatus> for DeviceStatus {
    fn from(device: proto::DeviceStatus) -> Self {
        match device {
            proto::DeviceStatus::Feeder(feeder) => Self::Feeder(feeder.into()),
        }
    }
}

impl From<proto::FeederStatus> for FeederStatus {
    fn from(status: proto::FeederStatus) -> Self {
        let proto::FeederStatus {
            locked,
            settings_generation,
            food,
            feed_fault,
            desiccant,
            blocked,
            supply,
            ext: _,
        } = status;
        Self {
            locked,
            settings_generation: Generation::settings(settings_generation),
            food: lenient(food, FoodLevel::Unknown),
            feed_fault: feed_fault.map(|fault| FeedFault {
                feed: fault.feed,
                outcome: lenient(fault.outcome, FeedOutcome::Unknown),
                reason: lenient(fault.reason, FeedReason::Unknown),
            }),
            readiness: match blocked {
                None => Readiness::Ready,
                Some(blocked) => Readiness::Blocked(lenient(blocked, Blocked::Unknown)),
            },
            desiccant: desiccant.into(),
            supply: supply.into(),
        }
    }
}

impl From<proto::Desiccant> for Desiccant {
    fn from(desiccant: proto::Desiccant) -> Self {
        match desiccant {
            proto::Desiccant::Unset => Self::Unset,
            proto::Desiccant::Unknown => Self::Unknown,
            proto::Desiccant::Remaining { days } => Self::Remaining {
                days,
                due: desiccant.due() == Some(true),
            },
        }
    }
}

impl From<proto::Blocked> for Blocked {
    fn from(blocked: proto::Blocked) -> Self {
        match blocked {
            proto::Blocked::Starting => Self::Starting,
            proto::Blocked::Journal => Self::Journal,
            proto::Blocked::Fault => Self::Fault,
            proto::Blocked::Resolving => Self::Resolving,
            proto::Blocked::Feeding => Self::Feeding,
            proto::Blocked::Joining => Self::Joining,
            proto::Blocked::Updating => Self::Updating,
            proto::Blocked::Setup => Self::Setup,
            proto::Blocked::Unserved => Self::Unserved,
            proto::Blocked::Battery => Self::Battery,
            proto::Blocked::Restarting => Self::Restarting,
        }
    }
}

impl TryFrom<Blocked> for proto::Blocked {
    type Error = NoWireForm;

    fn try_from(blocked: Blocked) -> Result<Self, NoWireForm> {
        Ok(match blocked {
            Blocked::Starting => Self::Starting,
            Blocked::Journal => Self::Journal,
            Blocked::Fault => Self::Fault,
            Blocked::Resolving => Self::Resolving,
            Blocked::Feeding => Self::Feeding,
            Blocked::Joining => Self::Joining,
            Blocked::Updating => Self::Updating,
            Blocked::Setup => Self::Setup,
            Blocked::Unserved => Self::Unserved,
            Blocked::Battery => Self::Battery,
            Blocked::Restarting => Self::Restarting,
            Blocked::Unknown => return Err(NoWireForm),
        })
    }
}

impl From<proto::WifiSource> for WifiSource {
    fn from(source: proto::WifiSource) -> Self {
        match source {
            proto::WifiSource::Build => Self::Build,
            proto::WifiSource::Stored => Self::Stored,
            proto::WifiSource::Fallback => Self::Fallback,
        }
    }
}

impl From<proto::Power> for Power {
    fn from(power: proto::Power) -> Self {
        let proto::Power {
            adapter,
            battery_mv,
        } = power;
        Self {
            adapter,
            battery_mv,
        }
    }
}

impl From<proto::Battery> for Battery {
    fn from(battery: proto::Battery) -> Self {
        match battery {
            proto::Battery::Absent => Self::Absent,
            proto::Battery::Reading { mv, percent } => Self::Reading { mv, percent },
            proto::Battery::Implausible { mv } => Self::Implausible { mv },
            proto::Battery::Unread => Self::Unread,
            proto::Battery::Stale { age_ms } => Self::Stale { age_ms },
        }
    }
}

impl TryFrom<Battery> for proto::Battery {
    type Error = NoWireForm;

    fn try_from(battery: Battery) -> Result<Self, NoWireForm> {
        Ok(match battery {
            Battery::Absent => Self::Absent,
            Battery::Reading { mv, percent } => Self::Reading { mv, percent },
            Battery::Implausible { mv } => Self::Implausible { mv },
            Battery::Unread => Self::Unread,
            Battery::Stale { age_ms } => Self::Stale { age_ms },
            Battery::Unknown => return Err(NoWireForm),
        })
    }
}

impl From<proto::Supply> for Supply {
    fn from(supply: proto::Supply) -> Self {
        let proto::Supply {
            adapter,
            battery,
            below_floor,
        } = supply;
        Self {
            adapter,
            battery: battery.into(),
            below_floor,
        }
    }
}

impl From<proto::Settings> for FeederSettings {
    fn from(settings: proto::Settings) -> Self {
        let proto::Settings {
            generation,
            lights,
            sound,
            volume,
            lights_window,
            sound_window,
            desiccant_days,
            desiccant_reset,
            meal_reminder,
            writable,
            ext: _,
        } = settings;
        Self {
            generation: Generation::settings(generation),
            lights,
            sound,
            volume,
            lights_window: lights_window.into(),
            sound_window: sound_window.into(),
            desiccant_days,
            desiccant_replaced: desiccant_reset.and_then(calendar_day),
            meal_reminder: meal_reminder.into(),
            writable,
        }
    }
}

/// The calendar date of a feeder's local day number (days since
/// 1970-01-01); `None` for one past jiff's range, which no valid clock
/// gives.
fn calendar_day(day: i32) -> Option<jiff::civil::Date> {
    jiff::civil::date(1970, 1, 1)
        .checked_add(jiff::Span::new().try_days(day).ok()?)
        .ok()
}

impl From<proto::MealReminder> for MealReminder {
    fn from(reminder: proto::MealReminder) -> Self {
        match reminder {
            proto::MealReminder::Off => Self::Off,
            proto::MealReminder::Before { minutes } => Self::Before { minutes },
        }
    }
}

impl TryFrom<MealReminder> for proto::MealReminder {
    type Error = NoWireForm;

    fn try_from(reminder: MealReminder) -> Result<Self, NoWireForm> {
        Ok(match reminder {
            MealReminder::Off => Self::Off,
            MealReminder::Before { minutes } => Self::Before { minutes },
            MealReminder::Unknown => return Err(NoWireForm),
        })
    }
}

impl From<proto::Window> for Window {
    fn from(window: proto::Window) -> Self {
        match window {
            proto::Window::Always => Self::Always,
            proto::Window::Between { start, end } => Self::Between { start, end },
        }
    }
}

impl TryFrom<Window> for proto::Window {
    type Error = NoWireForm;

    fn try_from(window: Window) -> Result<Self, NoWireForm> {
        Ok(match window {
            Window::Always => Self::Always,
            Window::Between { start, end } => Self::Between { start, end },
            Window::Unknown => return Err(NoWireForm),
        })
    }
}

impl From<proto::EventBody> for EventBody {
    fn from(body: proto::EventBody) -> Self {
        match body {
            proto::EventBody::Boot { bank, reset_flags } => {
                EventBody::Core(CoreEvent::Boot { bank, reset_flags })
            }
            proto::EventBody::Lock { locked, by } => EventBody::Feeder(FeederEvent::Lock {
                locked,
                by: by.into(),
            }),
            proto::EventBody::Settings { generation, by } => {
                EventBody::Feeder(FeederEvent::Settings {
                    generation: Generation(generation),
                    by: by.into(),
                })
            }
            proto::EventBody::Wifi { generation, by } => EventBody::Core(CoreEvent::Wifi {
                generation: Generation(generation),
                by: by.into(),
            }),
            proto::EventBody::Enrolled { client, role, path } => {
                EventBody::Core(CoreEvent::Enrolled {
                    client: Hex(client),
                    role: role.into(),
                    path: path.into(),
                })
            }
            proto::EventBody::Revoked { client, by } => EventBody::Core(CoreEvent::Revoked {
                client: Hex(client),
                by: by.into(),
            }),
            proto::EventBody::ClockSync { by, jump_ms } => EventBody::Core(CoreEvent::ClockSync {
                by: by.into(),
                jump_ms,
            }),
            proto::EventBody::Lost { count } => EventBody::Core(CoreEvent::Lost { count }),
            proto::EventBody::UpdateStop { stop } => {
                EventBody::Core(CoreEvent::UpdateStop { stop: stop.into() })
            }
            proto::EventBody::UpdateAtBoot { candidate } => {
                EventBody::Core(CoreEvent::UpdateAtBoot {
                    candidate: candidate.into(),
                })
            }
            proto::EventBody::Restarting => EventBody::Core(CoreEvent::Restarting),
            proto::EventBody::UpdateConfirmed => EventBody::Core(CoreEvent::UpdateConfirmed),
            proto::EventBody::Invited { client, by } => EventBody::Core(CoreEvent::Invited {
                client: Hex(client),
                by: Hex(by),
            }),
            proto::EventBody::Fed {
                feed,
                requested,
                completed,
                outcome,
                reason,
                hopper,
                by,
            } => EventBody::Feeder(FeederEvent::Fed {
                feed,
                requested,
                completed,
                outcome: outcome.into(),
                reason: reason.into(),
                hopper: hopper.into(),
                by: by.into(),
            }),
            proto::EventBody::FeedingBlocked { reason } => {
                EventBody::Feeder(FeederEvent::FeedingBlocked {
                    reason: reason.map(Blocked::from),
                })
            }
            proto::EventBody::Supply(proto::Supply {
                adapter,
                battery,
                below_floor,
            }) => EventBody::Feeder(FeederEvent::Supply {
                adapter,
                battery: battery.into(),
                below_floor,
            }),
            proto::EventBody::Plans { generation, by } => EventBody::Feeder(FeederEvent::Plans {
                generation: Generation(generation),
                by: by.into(),
            }),
            proto::EventBody::Missed { plan, due, reason } => {
                EventBody::Feeder(FeederEvent::Missed {
                    plan,
                    due: seconds(due),
                    reason: reason.into(),
                })
            }
            proto::EventBody::DispenseRefused { reason } => {
                EventBody::Feeder(FeederEvent::DispenseRefused {
                    reason: reason.into(),
                })
            }
            proto::EventBody::MealSoon {
                plan,
                due,
                minute,
                portions,
            } => EventBody::Feeder(FeederEvent::MealSoon {
                plan,
                due: seconds(due),
                minute,
                portions,
            }),
            proto::EventBody::Food { level } => EventBody::Feeder(FeederEvent::Food {
                level: level.into(),
            }),
            proto::EventBody::Skipped { plan, due, by } => {
                EventBody::Feeder(FeederEvent::Skipped {
                    plan,
                    due: seconds(due),
                    by: by.into(),
                })
            }
            proto::EventBody::Unskipped { plan, due, by } => {
                EventBody::Feeder(FeederEvent::Unskipped {
                    plan,
                    due: seconds(due),
                    by: by.into(),
                })
            }
        }
    }
}

/// The wire form, which is also how the server stores a body: the
/// feeder's own encoding, so a stored history decodes the way the feeder
/// meant it.
impl TryFrom<EventBody> for proto::EventBody {
    type Error = NoWireForm;

    fn try_from(body: EventBody) -> Result<Self, NoWireForm> {
        Ok(match body {
            EventBody::Core(CoreEvent::Boot { bank, reset_flags }) => {
                Self::Boot { bank, reset_flags }
            }
            EventBody::Feeder(FeederEvent::Lock { locked, by }) => Self::Lock {
                locked,
                by: by.try_into()?,
            },
            EventBody::Feeder(FeederEvent::Settings { generation, by }) => Self::Settings {
                generation: generation.0,
                by: by.try_into()?,
            },
            EventBody::Core(CoreEvent::Wifi { generation, by }) => Self::Wifi {
                generation: generation.0,
                by: by.try_into()?,
            },
            EventBody::Core(CoreEvent::Enrolled { client, role, path }) => Self::Enrolled {
                client: client.0,
                role: role.try_into()?,
                path: path.try_into()?,
            },
            EventBody::Core(CoreEvent::Revoked { client, by }) => Self::Revoked {
                client: client.0,
                by: by.try_into()?,
            },
            EventBody::Core(CoreEvent::ClockSync { by, jump_ms }) => Self::ClockSync {
                by: by.try_into()?,
                jump_ms,
            },
            EventBody::Core(CoreEvent::Lost { count }) => Self::Lost { count },
            EventBody::Core(CoreEvent::UpdateStop { stop }) => Self::UpdateStop {
                stop: stop.try_into()?,
            },
            EventBody::Core(CoreEvent::UpdateAtBoot { candidate }) => Self::UpdateAtBoot {
                candidate: candidate.try_into()?,
            },
            EventBody::Core(CoreEvent::Restarting) => Self::Restarting,
            EventBody::Core(CoreEvent::UpdateConfirmed) => Self::UpdateConfirmed,
            EventBody::Core(CoreEvent::Invited { client, by }) => Self::Invited {
                client: client.0,
                by: by.0,
            },
            EventBody::Feeder(FeederEvent::Fed {
                feed,
                requested,
                completed,
                outcome,
                reason,
                hopper,
                by,
            }) => Self::Fed {
                feed,
                requested,
                completed,
                outcome: outcome.try_into()?,
                reason: reason.try_into()?,
                hopper: hopper.try_into()?,
                by: by.try_into()?,
            },
            EventBody::Feeder(FeederEvent::FeedingBlocked { reason }) => Self::FeedingBlocked {
                reason: reason.map(TryInto::try_into).transpose()?,
            },
            EventBody::Feeder(FeederEvent::Supply {
                adapter,
                battery,
                below_floor,
            }) => Self::Supply(proto::Supply {
                adapter,
                battery: battery.try_into()?,
                below_floor,
            }),
            EventBody::Feeder(FeederEvent::Plans { generation, by }) => Self::Plans {
                generation: generation.0,
                by: by.try_into()?,
            },
            // A due time came from the feeder's u32 seconds.
            EventBody::Feeder(FeederEvent::Missed { plan, due, reason }) => Self::Missed {
                plan,
                due: u32::try_from(due.as_second()).unwrap_or(u32::MAX),
                reason: reason.try_into()?,
            },
            EventBody::Feeder(FeederEvent::DispenseRefused { reason }) => Self::DispenseRefused {
                reason: reason.try_into()?,
            },
            EventBody::Feeder(FeederEvent::MealSoon {
                plan,
                due,
                minute,
                portions,
            }) => Self::MealSoon {
                plan,
                due: u32::try_from(due.as_second()).unwrap_or(u32::MAX),
                minute,
                portions,
            },
            EventBody::Feeder(FeederEvent::Food { level }) => Self::Food {
                level: level.try_into()?,
            },
            EventBody::Feeder(FeederEvent::Skipped { plan, due, by }) => Self::Skipped {
                plan,
                due: u32::try_from(due.as_second()).unwrap_or(u32::MAX),
                by: by.try_into()?,
            },
            EventBody::Feeder(FeederEvent::Unskipped { plan, due, by }) => Self::Unskipped {
                plan,
                due: u32::try_from(due.as_second()).unwrap_or(u32::MAX),
                by: by.try_into()?,
            },
        })
    }
}

impl From<proto::FoodLevel> for FoodLevel {
    fn from(level: proto::FoodLevel) -> Self {
        match level {
            proto::FoodLevel::Unread => Self::Unread,
            proto::FoodLevel::Present => Self::Present,
            proto::FoodLevel::Low => Self::Low,
        }
    }
}

impl TryFrom<FoodLevel> for proto::FoodLevel {
    type Error = NoWireForm;

    fn try_from(level: FoodLevel) -> Result<Self, NoWireForm> {
        Ok(match level {
            FoodLevel::Unread => Self::Unread,
            FoodLevel::Present => Self::Present,
            FoodLevel::Low => Self::Low,
            FoodLevel::Unknown => return Err(NoWireForm),
        })
    }
}

impl From<proto::Source> for Source {
    fn from(source: proto::Source) -> Self {
        match source {
            proto::Source::Panel => Self::Panel,
            proto::Source::Client(client) => Self::Client {
                client: Hex(client),
            },
            proto::Source::Feeder => Self::Feeder,
            proto::Source::Schedule(plan) => Self::Schedule { plan },
        }
    }
}

impl TryFrom<Source> for proto::Source {
    type Error = NoWireForm;

    fn try_from(source: Source) -> Result<Self, NoWireForm> {
        Ok(match source {
            Source::Panel => Self::Panel,
            Source::Client { client } => Self::Client(client.0),
            Source::Feeder => Self::Feeder,
            Source::Schedule { plan } => Self::Schedule(plan),
            Source::Unknown => return Err(NoWireForm),
        })
    }
}

impl From<proto::MissedReason> for MissedReason {
    fn from(reason: proto::MissedReason) -> Self {
        match reason {
            proto::MissedReason::Late => Self::Late,
            proto::MissedReason::Blocked(reason) => Self::Blocked {
                reason: reason.into(),
            },
            proto::MissedReason::Interrupted => Self::Interrupted,
        }
    }
}

impl TryFrom<MissedReason> for proto::MissedReason {
    type Error = NoWireForm;

    fn try_from(reason: MissedReason) -> Result<Self, NoWireForm> {
        Ok(match reason {
            MissedReason::Late => Self::Late,
            MissedReason::Blocked { reason } => Self::Blocked(reason.try_into()?),
            MissedReason::Interrupted => Self::Interrupted,
            MissedReason::Unknown => return Err(NoWireForm),
        })
    }
}

impl From<proto::Plan> for Plan {
    fn from(plan: proto::Plan) -> Self {
        // A tail a newer build filled holds nothing this build can show,
        // and `PutPlans` never writes one back.
        let proto::Plan {
            minute,
            portions,
            days,
            enabled,
            call,
            ext: _,
        } = plan;
        Self {
            minute,
            portions,
            days: Weekdays::from_bits(days),
            enabled,
            call,
        }
    }
}

impl From<Plan> for proto::Plan {
    fn from(plan: Plan) -> Self {
        let Plan {
            minute,
            portions,
            days,
            enabled,
            call,
        } = plan;
        Self {
            minute,
            portions,
            days: days.bits(),
            enabled,
            call,
            ext: proto::Tail::EMPTY,
        }
    }
}

impl From<proto::CallClip> for CallClip {
    fn from(clip: proto::CallClip) -> Self {
        match clip {
            proto::CallClip::None => Self::None,
            proto::CallClip::Receiving {
                bytes,
                crc,
                received,
            } => Self::Receiving {
                bytes,
                crc,
                received,
            },
            proto::CallClip::Stored { bytes, crc } => Self::Stored { bytes, crc },
        }
    }
}

impl From<proto::CallView> for CallView {
    fn from(view: proto::CallView) -> Self {
        let proto::CallView { writable, clip } = view;
        Self {
            writable,
            clip: clip.into(),
        }
    }
}

impl From<proto::Role> for ClientRole {
    fn from(role: proto::Role) -> Self {
        match role {
            proto::Role::Owner => Self::Owner,
            proto::Role::Member => Self::Member,
            proto::Role::Server => Self::Server,
            proto::Role::UpdateSigner => Self::UpdateSigner,
        }
    }
}

impl TryFrom<ClientRole> for proto::Role {
    type Error = NoWireForm;

    fn try_from(role: ClientRole) -> Result<Self, NoWireForm> {
        Ok(match role {
            ClientRole::Owner => Self::Owner,
            ClientRole::Member => Self::Member,
            ClientRole::Server => Self::Server,
            ClientRole::UpdateSigner => Self::UpdateSigner,
            ClientRole::Unknown => return Err(NoWireForm),
        })
    }
}

impl From<proto::Path> for EnrollPath {
    fn from(path: proto::Path) -> Self {
        match path {
            proto::Path::Setup => Self::Setup,
            proto::Path::Invite => Self::Invite,
            proto::Path::Recovery => Self::Recovery,
            proto::Path::Press => Self::Press,
        }
    }
}

impl TryFrom<EnrollPath> for proto::Path {
    type Error = NoWireForm;

    fn try_from(path: EnrollPath) -> Result<Self, NoWireForm> {
        Ok(match path {
            EnrollPath::Setup => Self::Setup,
            EnrollPath::Invite => Self::Invite,
            EnrollPath::Recovery => Self::Recovery,
            EnrollPath::Press => Self::Press,
            EnrollPath::Unknown => return Err(NoWireForm),
        })
    }
}

impl From<proto::FeedOutcome> for FeedOutcome {
    fn from(outcome: proto::FeedOutcome) -> Self {
        match outcome {
            proto::FeedOutcome::Done => Self::Done,
            proto::FeedOutcome::Cancelled => Self::Cancelled,
            proto::FeedOutcome::Expired => Self::Expired,
            proto::FeedOutcome::Failed => Self::Failed,
            proto::FeedOutcome::Uncertain => Self::Uncertain,
        }
    }
}

impl TryFrom<FeedOutcome> for proto::FeedOutcome {
    type Error = NoWireForm;

    fn try_from(outcome: FeedOutcome) -> Result<Self, NoWireForm> {
        Ok(match outcome {
            FeedOutcome::Done => Self::Done,
            FeedOutcome::Cancelled => Self::Cancelled,
            FeedOutcome::Expired => Self::Expired,
            FeedOutcome::Failed => Self::Failed,
            FeedOutcome::Uncertain => Self::Uncertain,
            FeedOutcome::Unknown => return Err(NoWireForm),
        })
    }
}

impl From<proto::FeedReason> for FeedReason {
    fn from(reason: proto::FeedReason) -> Self {
        match reason {
            proto::FeedReason::Done => Self::Done,
            proto::FeedReason::Cancelled => Self::Cancelled,
            proto::FeedReason::Clock => Self::Clock,
            proto::FeedReason::ObservationGap => Self::ObservationGap,
            proto::FeedReason::Timeout => Self::Timeout,
            proto::FeedReason::NoFeedback => Self::NoFeedback,
            proto::FeedReason::Power => Self::Power,
            proto::FeedReason::Sensor => Self::Sensor,
            proto::FeedReason::OutletBlocked => Self::OutletBlocked,
            proto::FeedReason::Jammed => Self::Jammed,
            proto::FeedReason::MarkStuck => Self::MarkStuck,
            proto::FeedReason::Overcurrent => Self::Overcurrent,
            proto::FeedReason::FeedbackStream => Self::FeedbackStream,
            proto::FeedReason::Output => Self::Output,
            proto::FeedReason::Interrupted => Self::Interrupted,
        }
    }
}

impl TryFrom<FeedReason> for proto::FeedReason {
    type Error = NoWireForm;

    fn try_from(reason: FeedReason) -> Result<Self, NoWireForm> {
        Ok(match reason {
            FeedReason::Done => Self::Done,
            FeedReason::Cancelled => Self::Cancelled,
            FeedReason::Clock => Self::Clock,
            FeedReason::ObservationGap => Self::ObservationGap,
            FeedReason::Timeout => Self::Timeout,
            FeedReason::NoFeedback => Self::NoFeedback,
            FeedReason::Power => Self::Power,
            FeedReason::Sensor => Self::Sensor,
            FeedReason::OutletBlocked => Self::OutletBlocked,
            FeedReason::Jammed => Self::Jammed,
            FeedReason::MarkStuck => Self::MarkStuck,
            FeedReason::Overcurrent => Self::Overcurrent,
            FeedReason::FeedbackStream => Self::FeedbackStream,
            FeedReason::Output => Self::Output,
            FeedReason::Interrupted => Self::Interrupted,
            FeedReason::Unknown => return Err(NoWireForm),
        })
    }
}

impl From<proto::Sample> for Sample {
    fn from(sample: proto::Sample) -> Self {
        let proto::Sample { at_ms, metric } = sample;
        Self {
            uptime_ms: at_ms,
            value: metric.into(),
        }
    }
}

impl From<proto::Metric> for SampleValue {
    fn from(metric: proto::Metric) -> Self {
        match metric {
            proto::Metric::Power(proto::Power {
                adapter,
                battery_mv,
            }) => Self::Power {
                adapter,
                battery_mv,
            },
            proto::Metric::Rssi(dbm) => Self::Rssi { dbm },
            proto::Metric::Heap { free, min } => Self::Heap { free, min },
            proto::Metric::Loop {
                passes,
                max_us,
                p99_us,
            } => Self::Loop {
                passes,
                max_us,
                p99_us,
            },
            proto::Metric::Gateway { round_trip_ms } => Self::Gateway { round_trip_ms },
            proto::Metric::Wifi {
                channel,
                ap_changes,
            } => Self::Wifi {
                channel,
                ap_changes,
            },
            proto::Metric::Supply(proto::Supply {
                adapter,
                battery,
                below_floor,
            }) => Self::Supply {
                adapter,
                battery: battery.into(),
                below_floor,
            },
        }
    }
}

/// The wire form, which is also how the server stores a sample's value.
impl TryFrom<SampleValue> for proto::Metric {
    type Error = NoWireForm;

    fn try_from(value: SampleValue) -> Result<Self, NoWireForm> {
        Ok(match value {
            SampleValue::Power {
                adapter,
                battery_mv,
            } => Self::Power(proto::Power {
                adapter,
                battery_mv,
            }),
            SampleValue::Rssi { dbm } => Self::Rssi(dbm),
            SampleValue::Heap { free, min } => Self::Heap { free, min },
            SampleValue::Loop {
                passes,
                max_us,
                p99_us,
            } => Self::Loop {
                passes,
                max_us,
                p99_us,
            },
            SampleValue::Gateway { round_trip_ms } => Self::Gateway { round_trip_ms },
            SampleValue::Wifi {
                channel,
                ap_changes,
            } => Self::Wifi {
                channel,
                ap_changes,
            },
            SampleValue::Supply {
                adapter,
                battery,
                below_floor,
            } => Self::Supply(proto::Supply {
                adapter,
                battery: battery.try_into()?,
                below_floor,
            }),
        })
    }
}

impl From<proto::Outcome<'_>> for Answer {
    fn from(outcome: proto::Outcome<'_>) -> Self {
        match outcome {
            proto::Outcome::Done { generation } => Self::Done {
                generation: generation.map(Generation),
            },
            proto::Outcome::Status(status) => Self::Status(status.into()),
            proto::Outcome::Feeder(proto::FeederOutcome::Settings(settings)) => {
                Self::Settings(Settings::Feeder(settings.into()))
            }
            proto::Outcome::Clients(proto::Clients { clients }) => Self::Clients(
                clients
                    .iter()
                    .flatten()
                    .map(
                        |&proto::Client {
                             key,
                             label,
                             role,
                             you,
                         }| Client {
                            key: Hex(key),
                            label: label.to_owned(),
                            role: role.into(),
                            you,
                        },
                    )
                    .collect(),
            ),
            proto::Outcome::Invite(proto::Invite {
                id,
                secret,
                seconds,
            }) => Self::Invite(Invite {
                id,
                secret: Hex(secret),
                seconds,
            }),
            proto::Outcome::Refused(refusal) => Self::Refused(refusal.into()),
            proto::Outcome::Feeder(proto::FeederOutcome::Feeding { feed }) => {
                Self::Feeding { feed }
            }
            proto::Outcome::Update(view) => Self::Update(view.into()),
            proto::Outcome::Time(view) => Self::Time(view.into()),
            proto::Outcome::Stats(stats) => Self::Stats(stats.into()),
            proto::Outcome::Feeder(proto::FeederOutcome::FeedTrace(page)) => {
                Self::FeedTrace(page.into())
            }
            proto::Outcome::Feeder(proto::FeederOutcome::Calibration(view)) => {
                Self::Calibration(view.into())
            }
            proto::Outcome::Bank1(proto::Bank1Page {
                offset,
                total,
                bytes,
            }) => Self::Bank1(Bank1Page {
                offset,
                total,
                bytes: bytes.to_vec(),
            }),
            proto::Outcome::Feeder(proto::FeederOutcome::Plans(proto::Plans {
                generation,
                writable,
                plans,
                skipped,
            })) => Self::Plans(Plans {
                generation: Generation(generation),
                writable,
                plans: plans.map(|plan| plan.map(Into::into)),
                skipped: SkippedMeals::from_seconds(skipped),
            }),
            proto::Outcome::Feeder(proto::FeederOutcome::Call(view)) => Self::Call(view.into()),
            proto::Outcome::Alerts(view) => Self::Alerts(view.into()),
        }
    }
}

impl From<proto::CurrentUnit> for CurrentUnit {
    fn from(unit: proto::CurrentUnit) -> Self {
        match unit {
            proto::CurrentUnit::Scaled => Self::Scaled,
            proto::CurrentUnit::MilliAmps => Self::MilliAmps,
        }
    }
}

impl From<proto::TraceSummary> for TraceSummary {
    fn from(summary: proto::TraceSummary) -> Self {
        let proto::TraceSummary {
            feed,
            requested,
            completed,
            outcome,
            reason,
            drive_ms,
            samples,
            unit,
            current_peak,
            current_mean,
            rising,
            falling,
            first_edge_ms,
            beam_breaks,
            reversals,
            battery_rest_mv,
            battery_min_mv,
            stride,
            truncated,
        } = summary;
        Self {
            feed,
            requested,
            completed,
            outcome: lenient(outcome, FeedOutcome::Unknown),
            reason: lenient(reason, FeedReason::Unknown),
            drive_ms,
            samples,
            unit: lenient(unit, CurrentUnit::Unknown),
            current_peak,
            current_mean,
            rising,
            falling,
            first_edge_ms,
            beam_breaks,
            reversals,
            battery_rest_mv,
            battery_min_mv,
            stride,
            truncated,
        }
    }
}

impl From<proto::TracePage<'_>> for TracePage {
    fn from(page: proto::TracePage<'_>) -> Self {
        let proto::TracePage {
            summary,
            stored,
            offset,
            total,
            bytes,
        } = page;
        Self {
            summary: summary.into(),
            stored,
            offset,
            total,
            bytes: bytes.to_vec(),
        }
    }
}

impl From<proto_trace::Entry> for TraceEntry {
    fn from(entry: proto_trace::Entry) -> Self {
        match entry {
            proto_trace::Entry::Current { at_ms, value } => Self::Current { at_ms, value },
            proto_trace::Entry::Mark { at_ms, edges, high } => Self::Mark { at_ms, edges, high },
            proto_trace::Entry::Beams {
                at_ms,
                outlet_clear,
                food,
            } => Self::Beams {
                at_ms,
                outlet_clear,
                food,
            },
            proto_trace::Entry::Battery { at_ms, mv } => Self::Battery { at_ms, mv },
            proto_trace::Entry::Motor { at_ms, motor } => Self::Motor {
                at_ms,
                motor: match motor {
                    proto_trace::Motor::Stop => TraceMotor::Stop,
                    proto_trace::Motor::Drive0 => TraceMotor::Drive0,
                    proto_trace::Motor::Drive1 => TraceMotor::Drive1,
                },
            },
        }
    }
}

impl Trace {
    /// A whole trace from its summary and its body's bytes.
    pub fn decode(summary: TraceSummary, stored: bool, body: &[u8]) -> Result<Self, Malformed> {
        let entries = proto_trace::Reader::new(body)
            .map(|entry| entry.map(TraceEntry::from).map_err(|_| Malformed))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            summary,
            stored,
            entries,
        })
    }
}

impl From<StatsSection> for proto::StatsSection {
    fn from(section: StatsSection) -> Self {
        match section {
            StatsSection::Counters => Self::Counters,
            StatsSection::Memory => Self::Memory,
            StatsSection::Flash => Self::Flash,
            StatsSection::Power => Self::Power,
            StatsSection::Food => Self::Food,
            StatsSection::Feeds => Self::Feeds,
            StatsSection::Portions => Self::Portions,
        }
    }
}

impl From<proto::CalibrationView> for CalibrationView {
    fn from(view: proto::CalibrationView) -> Self {
        let proto::CalibrationView {
            generation,
            writable,
            calibration,
            phase,
        } = view;
        Self {
            generation: Generation(generation),
            writable,
            calibration: calibration.into(),
            phase: phase.map(|phase| match phase {
                proto::Lenient::Known(proto::Phase::AtMark) => Phase::AtMark,
                proto::Lenient::Known(proto::Phase::Unknown) => Phase::Unknown,
                proto::Lenient::Unknown(_) => Phase::Newer,
            }),
        }
    }
}

impl From<proto::Calibration> for Calibration {
    fn from(calibration: proto::Calibration) -> Self {
        let proto::Calibration { portion_mg, bowl } = calibration;
        Self {
            portion_mg,
            bowl: match bowl {
                proto::Lenient::Known(proto::Bowl::Single) => Bowl::Single,
                proto::Lenient::Known(proto::Bowl::Double) => Bowl::Double,
                proto::Lenient::Unknown(_) => Bowl::Unknown,
            },
        }
    }
}

impl From<Calibration> for proto::Calibration {
    fn from(calibration: Calibration) -> Self {
        let Calibration { portion_mg, bowl } = calibration;
        Self {
            portion_mg,
            bowl: match bowl {
                Bowl::Single => proto::Bowl::Single.into(),
                Bowl::Double => proto::Bowl::Double.into(),
                // An index no build gives a mode: the device refuses any
                // mode but its own `Unverified`, so a calibration read from
                // a newer build and written back changes nothing.
                Bowl::Unknown => proto::Lenient::Unknown(u8::MAX),
            },
        }
    }
}

impl From<proto::Stats<'_>> for Stats {
    fn from(stats: proto::Stats<'_>) -> Self {
        match stats {
            proto::Stats::Counters(counters) => Self::Counters(counters.into()),
            proto::Stats::Memory(proto::Memory {
                heap_free,
                heap_min,
                tasks,
                tasks_total,
            }) => Self::Memory(Memory {
                tasks_total,
                heap_free,
                heap_min,
                tasks: tasks
                    .iter()
                    .flatten()
                    .map(|&proto::TaskStack { name, free_min }| TaskStack {
                        name: name.to_owned(),
                        free_min,
                    })
                    .collect(),
            }),
            proto::Stats::Flash(proto::Flash {
                boot,
                lifetime,
                events_kept,
                events_capacity,
            }) => Self::Flash(Flash {
                boot: boot.into(),
                lifetime: lifetime.map(Erases::from),
                events_kept,
                events_capacity,
            }),
            proto::Stats::Power(proto::PowerStats {
                adapter,
                adapter_changes,
                outages,
                battery_s,
                battery_min_mv,
                battery_max_mv,
                battery_mv,
                period_s,
            }) => Self::Power(PowerStats {
                adapter,
                adapter_changes,
                outages,
                battery_s,
                battery_min_mv,
                battery_max_mv,
                battery_mv: battery_mv.to_vec(),
                period_s,
            }),
            proto::Stats::Food(proto::Food {
                present,
                ran_low,
                refilled,
                feeds_since_refill,
            }) => Self::Food(Food {
                present,
                ran_low,
                refilled,
                feeds_since_refill,
            }),
            proto::Stats::Feeds(proto::FeedStats {
                traced,
                drive_ms,
                samples,
                unit,
                current_peak,
                current_mean,
                rising,
                falling,
                beam_breaks,
                reversals,
                kept,
                capacity,
                unsaved,
                last,
            }) => Self::Feeds(FeedStats {
                traced,
                drive_ms,
                samples,
                unit: lenient(unit, CurrentUnit::Unknown),
                current_peak,
                current_mean,
                rising,
                falling,
                beam_breaks,
                reversals,
                kept,
                capacity,
                unsaved,
                last: last.map(TraceSummary::from),
            }),
            proto::Stats::Portions(proto::Portions {
                portions,
                homed,
                outlet_breaks,
                portion_mg,
                delivered_mg,
            }) => Self::Portions(Portions {
                portions,
                homed,
                outlet_breaks,
                portion_mg,
                delivered_mg,
            }),
        }
    }
}

impl From<proto::Counters> for Counters {
    fn from(counters: proto::Counters) -> Self {
        let proto::Counters {
            boot,
            reset,
            lifetime,
            resets,
            handshake_max_ms,
            handshake_mean_ms,
        } = counters;
        Self {
            boot: boot.into(),
            reset: match reset.known() {
                Some(proto::ResetCause::PowerOn) => ResetCause::PowerOn,
                Some(proto::ResetCause::Watchdog) => ResetCause::Watchdog,
                Some(proto::ResetCause::Software) => ResetCause::Software,
                Some(proto::ResetCause::Brownout) => ResetCause::Brownout,
                Some(proto::ResetCause::Other) => ResetCause::Other,
                None => ResetCause::Unknown,
            },
            lifetime: lifetime.map(Counts::from),
            resets: resets.map(
                |proto::Resets {
                     power_on,
                     watchdog,
                     software,
                     brownout,
                     other,
                 }| Resets {
                    power_on,
                    watchdog,
                    software,
                    brownout,
                    other,
                },
            ),
            handshake_max_ms,
            handshake_mean_ms,
        }
    }
}

impl From<proto::Counts> for Counts {
    fn from(counts: proto::Counts) -> Self {
        let proto::Counts {
            uptime_s,
            feeds,
            feeds_failed,
            portions,
            presses,
            sessions,
            reconnects,
            dhcp_renewals,
            outages,
            battery_s,
        } = counts;
        Self {
            uptime_s,
            feeds,
            feeds_failed,
            portions,
            presses,
            sessions,
            reconnects,
            dhcp_renewals,
            outages,
            battery_s,
        }
    }
}

impl From<proto::Erases> for Erases {
    fn from(erases: proto::Erases) -> Self {
        let proto::Erases { storage, update } = erases;
        Self {
            storage: storage.to_vec(),
            update,
        }
    }
}

impl From<proto::TimeView<'_>> for TimeView {
    fn from(view: proto::TimeView<'_>) -> Self {
        let proto::TimeView {
            generation,
            writable,
            zone,
            ntp,
            clock_ms,
            error_ms,
            offset_s,
            rate_ppb,
            rate_bound_ppb,
            learned,
            syncs,
            last,
            last_age_s,
            ntp_last,
        } = view;
        Self {
            generation: Generation(generation),
            writable,
            zone: (!zone.is_empty()).then(|| zone.to_owned()),
            ntp: ntp.into(),
            clock_ms,
            error_ms,
            offset_s,
            rate_ppb,
            rate_bound_ppb,
            learned,
            syncs,
            last: last.map(|source| match source.known() {
                Some(proto::TimeSource::Owner) => TimeSource::Owner,
                Some(proto::TimeSource::Client) => TimeSource::Client,
                Some(proto::TimeSource::Ntp) => TimeSource::Ntp,
                None => TimeSource::Unknown,
            }),
            last_age_s,
            ntp_last: ntp_last.map(|result| match result.known() {
                Some(proto::NtpResult::Used) => NtpResult::Used,
                Some(proto::NtpResult::Bounded) => NtpResult::Bounded,
                Some(proto::NtpResult::Lost) => NtpResult::Lost,
                Some(proto::NtpResult::Refused) => NtpResult::Refused,
                None => NtpResult::Unknown,
            }),
        }
    }
}

impl From<proto::NtpServer> for NtpServer {
    fn from(ntp: proto::NtpServer) -> Self {
        match ntp {
            proto::NtpServer::Off => Self::Off,
            proto::NtpServer::Gateway { port } => Self::Gateway { port },
            proto::NtpServer::Address { ip, port } => Self::Address {
                ip: ip.into(),
                port,
            },
        }
    }
}

impl From<NtpServer> for proto::NtpServer {
    fn from(ntp: NtpServer) -> Self {
        match ntp {
            NtpServer::Off => Self::Off,
            NtpServer::Gateway { port } => Self::Gateway { port },
            NtpServer::Address { ip, port } => Self::Address {
                ip: ip.octets(),
                port,
            },
        }
    }
}

impl From<proto::UpdateView> for UpdateView {
    fn from(view: proto::UpdateView) -> Self {
        let proto::UpdateView {
            phase,
            bank,
            min_version,
            digest,
            version,
            image_len,
            written,
            wanted,
            stopped,
            bank0,
        } = view;
        Self {
            phase: phase.into(),
            bank,
            min_version,
            digest: digest.map(Hex),
            version,
            image_len,
            written,
            wanted: wanted.into_iter().flatten().collect(),
            stopped: stopped.map(UpdateStop::from),
            bank0: bank0.into(),
        }
    }
}

impl From<proto::Bank0Blocked> for Bank0Blocked {
    fn from(reason: proto::Bank0Blocked) -> Self {
        match reason {
            proto::Bank0Blocked::Marker => Self::Marker,
            proto::Bank0Blocked::Unrecorded => Self::Unrecorded,
            proto::Bank0Blocked::Key => Self::Key,
            proto::Bank0Blocked::Extent => Self::Extent,
            proto::Bank0Blocked::Changed => Self::Changed,
            proto::Bank0Blocked::Read => Self::Read,
        }
    }
}

impl From<proto::Bank0Check> for Bank0Check {
    fn from(check: proto::Bank0Check) -> Self {
        match check {
            proto::Bank0Check::Unchecked => Self::Unchecked,
            proto::Bank0Check::Match => Self::Match,
            proto::Bank0Check::Blocked { reason } => Self::Blocked(reason.into()),
        }
    }
}

impl From<proto::UpdatePhase> for UpdatePhase {
    fn from(phase: proto::UpdatePhase) -> Self {
        match phase {
            proto::UpdatePhase::Idle => Self::Idle,
            proto::UpdatePhase::Checking => Self::Checking,
            proto::UpdatePhase::GivingWay => Self::GivingWay,
            proto::UpdatePhase::Receiving => Self::Receiving,
            proto::UpdatePhase::Activated => Self::Activated,
        }
    }
}

impl From<proto::AlertsView> for AlertsView {
    fn from(view: proto::AlertsView) -> Self {
        let proto::AlertsView {
            epoch,
            generation,
            more,
            settling,
            alerts: proto::AlertTable { entries, skipped },
        } = view;
        Self {
            epoch,
            generation: Generation(generation),
            more,
            settling,
            alerts: entries.iter().flatten().map(OpenAlert::from).collect(),
            unread: skipped,
        }
    }
}

impl From<&proto::OpenAlert> for OpenAlert {
    fn from(open: &proto::OpenAlert) -> Self {
        let proto::OpenAlert { at, alert } = *open;
        Self {
            at: at.map(|at| seconds(at.get())),
            alert: alert.into(),
        }
    }
}

impl From<proto::Alert> for Alert {
    fn from(alert: proto::Alert) -> Self {
        match alert {
            proto::Alert::MealFailed {
                feed,
                plan,
                requested,
                completed,
                outcome,
                reason,
            } => Self::MealFailed {
                feed,
                plan,
                requested,
                completed,
                outcome: outcome.into(),
                reason: reason.into(),
            },
            proto::Alert::MealUncertain {
                feed,
                plan,
                requested,
                completed,
                reason,
            } => Self::MealUncertain {
                feed,
                plan,
                requested,
                completed,
                reason: reason.into(),
            },
            proto::Alert::MealMissed { plan, due, reason } => Self::MealMissed {
                plan,
                due: seconds(due),
                reason: reason.into(),
            },
            proto::Alert::MealNoRecord { plan, due } => Self::MealNoRecord {
                plan,
                due: seconds(due),
            },
            proto::Alert::PowerWasOut => Self::PowerWasOut,
            proto::Alert::UpdateFailed { stop } => Self::UpdateFailed { stop: stop.into() },
            proto::Alert::RolledBack => Self::RolledBack,
            proto::Alert::ClientJoined { client, role, path } => Self::ClientJoined {
                client: Hex(client),
                role: role.into(),
                path: path.into(),
            },
            proto::Alert::ClientRevoked { client, by } => Self::ClientRevoked {
                client: Hex(client),
                by: by.into(),
            },
            proto::Alert::RepeatedRestarts => Self::RepeatedRestarts,
            proto::Alert::NoClock => Self::NoClock,
            proto::Alert::CannotFeed { reason } => Self::CannotFeed {
                reason: reason.into(),
            },
            proto::Alert::FoodLow => Self::FoodLow,
            proto::Alert::FoodLikelyOut { confirmed } => Self::FoodLikelyOut { confirmed },
            proto::Alert::DesiccantDue { days } => Self::DesiccantDue { days },
            proto::Alert::BatteryBelowFloor => Self::BatteryBelowFloor,
            proto::Alert::PlansStuck => Self::PlansStuck,
        }
    }
}

impl TryFrom<UpdateCandidate> for proto::UpdateCandidate {
    type Error = NoWireForm;

    fn try_from(candidate: UpdateCandidate) -> Result<Self, NoWireForm> {
        Ok(match candidate {
            UpdateCandidate::None => Self::None,
            UpdateCandidate::Pending => Self::Pending,
            UpdateCandidate::Activated => Self::Activated,
            UpdateCandidate::Unknown => return Err(NoWireForm),
        })
    }
}

impl From<proto::UpdateCandidate> for UpdateCandidate {
    fn from(candidate: proto::UpdateCandidate) -> Self {
        match candidate {
            proto::UpdateCandidate::None => Self::None,
            proto::UpdateCandidate::Pending => Self::Pending,
            proto::UpdateCandidate::Activated => Self::Activated,
        }
    }
}

impl TryFrom<UpdateStop> for proto::UpdateStop {
    type Error = NoWireForm;

    fn try_from(stop: UpdateStop) -> Result<Self, NoWireForm> {
        Ok(match stop {
            UpdateStop::Timeout => Self::Timeout,
            UpdateStop::Cancelled => Self::Cancelled,
            UpdateStop::Order => Self::Order,
            UpdateStop::Marker => Self::Marker,
            UpdateStop::Write => Self::Write,
            UpdateStop::Hash => Self::Hash,
            UpdateStop::Activate => Self::Activate,
            UpdateStop::Store => Self::Store,
            UpdateStop::Unknown => return Err(NoWireForm),
        })
    }
}

impl From<proto::UpdateStop> for UpdateStop {
    fn from(stop: proto::UpdateStop) -> Self {
        match stop {
            proto::UpdateStop::Timeout => Self::Timeout,
            proto::UpdateStop::Cancelled => Self::Cancelled,
            proto::UpdateStop::Order => Self::Order,
            proto::UpdateStop::Marker => Self::Marker,
            proto::UpdateStop::Write => Self::Write,
            proto::UpdateStop::Hash => Self::Hash,
            proto::UpdateStop::Activate => Self::Activate,
            proto::UpdateStop::Store => Self::Store,
        }
    }
}

impl From<proto::UpdateRefusal> for UpdateRefusal {
    fn from(refusal: proto::UpdateRefusal) -> Self {
        match refusal {
            proto::UpdateRefusal::Manifest => Self::Manifest,
            proto::UpdateRefusal::Key => Self::Key,
            proto::UpdateRefusal::Revoked => Self::Revoked,
            proto::UpdateRefusal::Signature => Self::Signature,
            proto::UpdateRefusal::Target => Self::Target,
            proto::UpdateRefusal::Same => Self::Same,
            proto::UpdateRefusal::Downgrade => Self::Downgrade,
            proto::UpdateRefusal::Statement => Self::Statement,
            proto::UpdateRefusal::Pending => Self::Pending,
            proto::UpdateRefusal::Failed => Self::Failed,
            proto::UpdateRefusal::Fallback { reason } => Self::Fallback {
                reason: reason.into(),
            },
            proto::UpdateRefusal::Attempt => Self::Attempt,
            proto::UpdateRefusal::Installing => Self::Installing,
            proto::UpdateRefusal::NotInstalling => Self::NotInstalling,
            proto::UpdateRefusal::Store => Self::Store,
            proto::UpdateRefusal::Unconfirmed => Self::Unconfirmed,
            proto::UpdateRefusal::Battery => Self::Battery,
            proto::UpdateRefusal::HeldAsSessionKey => Self::HeldAsSessionKey,
        }
    }
}

const _: () = assert!(
    crate::PORTIONS_MIN == proto::PORTIONS_MIN && crate::PORTIONS_MAX == proto::PORTIONS_MAX
);

impl From<proto::Refusal> for Refusal {
    fn from(refusal: proto::Refusal) -> Self {
        match refusal {
            proto::Refusal::Forbidden => Self::Forbidden,
            proto::Refusal::Stale { current } => Self::Stale {
                current: Generation(current),
            },
            proto::Refusal::Busy => Self::Busy,
            proto::Refusal::Unavailable => Self::Unavailable,
            proto::Refusal::Invalid => Self::Invalid,
            proto::Refusal::NotWritable => Self::NotWritable,
            proto::Refusal::Full => Self::Full,
            proto::Refusal::NotFound => Self::NotFound,
            proto::Refusal::Muted => Self::Muted,
            proto::Refusal::Time => Self::Time,
            proto::Refusal::Fault => Self::Fault,
            proto::Refusal::Update(refusal) => Self::Update(refusal.into()),
            proto::Refusal::Unverified => Self::Unverified,
            proto::Refusal::Portions { asked } => Self::Portions { asked },
            proto::Refusal::NoClock => Self::NoClock,
            proto::Refusal::Replaced => Self::Replaced,
        }
    }
}

impl Op {
    /// The kind of device whose module serves the op, `None` for the
    /// shared core every kind serves; another kind refuses it
    /// `Unavailable`. The variant says, so an op that holds a value only a
    /// newer build names (and has no wire form) still names its module,
    /// and the check costs no conversion. The wire form agrees
    /// (`an_op_s_kind_is_its_wire_form_s`).
    pub fn kind(&self) -> Option<DeviceKind> {
        match self {
            // A section of the shared statistics op may still be one kind's.
            Self::Stats { section } => section.kind(),
            Self::GetSettings
            | Self::PutSettings { .. }
            | Self::SetLock { .. }
            | Self::Play { .. }
            | Self::Feed { .. }
            | Self::CancelFeed { .. }
            | Self::FeedTrace { .. }
            | Self::GetCalibration
            | Self::PutCalibration { .. }
            | Self::GetPlans
            | Self::PutPlans { .. }
            | Self::ResetDesiccant
            | Self::GetCall
            | Self::StartCall { .. }
            | Self::CallChunk { .. }
            | Self::DeleteCall
            | Self::PlayCall
            | Self::SkipMeal { .. }
            | Self::UnskipMeal { .. }
            | Self::DeleteCallIf { .. } => Some(DeviceKind::Feeder),
            Self::Status
            | Self::PutWifi { .. }
            | Self::TimeSync(_)
            | Self::CreateInvite { .. }
            | Self::CreateBoundInvite { .. }
            | Self::ListClients
            | Self::RevokeClient { .. }
            | Self::Reboot
            | Self::OfferUpdate { .. }
            | Self::UpdateChunk { .. }
            | Self::UpdateState
            | Self::CancelUpdate
            | Self::EnrollUpdateKey { .. }
            | Self::UpdateKeyStatement { .. }
            | Self::GetTime
            | Self::PutTime { .. }
            | Self::TailLog { .. }
            | Self::ReadBank1 { .. }
            | Self::GetAlerts => None,
        }
    }

    /// Whether the op belongs to major 1's frozen part (the update ops,
    /// the keepalive and the invite), which a device of any build serves at
    /// the same indices: the only ops a client sends a pre-revision build
    /// ([`Skew::Legacy`](crate::Skew::Legacy)). A build of another revision
    /// is sent every op, since the API only grows.
    pub fn is_frozen(&self) -> bool {
        self.to_proto().is_ok_and(|op| op.is_frozen())
    }

    /// Whether a client holding `role` may send the op, by the device's own
    /// rules: first the matrix ([`wellfed_proto::access::allowed`]), which
    /// the device checks before anything else, then, for an invite, the role
    /// it grants ([`wellfed_proto::access::may_invite`]), which pairing
    /// checks next. A role only a newer build names grants nothing, and an
    /// op with no wire form is never sent.
    pub fn allowed(&self, role: ClientRole) -> bool {
        let (Ok(role), Ok(op)) = (proto::Role::try_from(role), self.to_proto()) else {
            return false;
        };
        let grants = match op {
            proto::Op::CreateInvite { role: granted }
            | proto::Op::CreateBoundInvite { role: granted, .. } => {
                wellfed_proto::access::may_invite(role, granted)
            }
            _ => true,
        };
        wellfed_proto::access::allowed(role, &op) && grants
    }

    /// The wire form, borrowing this op's strings; none for an op holding
    /// a value only a newer build names (a role or a window read from a
    /// newer server), which is never sent.
    pub fn to_proto(&self) -> Result<proto::Op<'_>, NoWireForm> {
        Ok(match self {
            Self::Status => proto::Op::Status,
            Self::GetSettings => proto::Op::Feeder(proto::FeederOp::GetSettings),
            Self::PutSettings { base, changes } => {
                proto::Op::Feeder(proto::FeederOp::PutSettings {
                    base: base.0,
                    changes: (*changes).try_into()?,
                })
            }
            Self::PutWifi { base, wifi } => proto::Op::PutWifi {
                base: base.0,
                wifi: wifi.as_ref().map(|wifi| proto::Wifi {
                    ssid: &wifi.ssid,
                    psk: &wifi.psk,
                }),
            },
            Self::SetLock { locked } => {
                proto::Op::Feeder(proto::FeederOp::SetLock { locked: *locked })
            }
            Self::Play { tune } => proto::Op::Feeder(proto::FeederOp::Play {
                tune: (*tune).into(),
            }),
            Self::TimeSync(sync) => proto::Op::TimeSync((*sync).into()),
            Self::CreateInvite { role } => proto::Op::CreateInvite {
                role: (*role).try_into()?,
            },
            Self::CreateBoundInvite { role, key } => proto::Op::CreateBoundInvite {
                role: (*role).try_into()?,
                key: *key,
            },
            Self::ListClients => proto::Op::ListClients,
            Self::RevokeClient { key } => proto::Op::RevokeClient { key: *key },
            Self::Feed { portions } => proto::Op::Feeder(proto::FeederOp::Feed {
                portions: *portions,
            }),
            Self::Reboot => proto::Op::Reboot,
            Self::CancelFeed { feed } => {
                proto::Op::Feeder(proto::FeederOp::CancelFeed { feed: *feed })
            }
            Self::OfferUpdate {
                manifest,
                downgrade,
            } => proto::Op::Update(proto::UpdateOp::Offer {
                manifest,
                downgrade: downgrade.as_deref(),
            }),
            Self::UpdateChunk { offset, data } => proto::Op::Update(proto::UpdateOp::Chunk {
                offset: *offset,
                data,
            }),
            Self::UpdateState => proto::Op::Update(proto::UpdateOp::State),
            Self::CancelUpdate => proto::Op::Update(proto::UpdateOp::Cancel),
            Self::EnrollUpdateKey { key, label } => {
                proto::Op::Update(proto::UpdateOp::EnrollKey { key: *key, label })
            }
            Self::UpdateKeyStatement { statement } => {
                proto::Op::Update(proto::UpdateOp::KeyStatement { statement })
            }
            Self::GetTime => proto::Op::GetTime,
            Self::PutTime { base, changes } => proto::Op::PutTime {
                base: base.0,
                changes: proto::TimeChanges {
                    zone: changes.zone.as_deref(),
                    ntp: changes.ntp.map(Into::into),
                },
            },
            Self::TailLog { after } => proto::Op::TailLog { after: *after },
            Self::Stats { section } => proto::Op::Stats {
                section: (*section).into(),
            },
            Self::FeedTrace { feed, offset } => proto::Op::Feeder(proto::FeederOp::FeedTrace {
                feed: *feed,
                offset: *offset,
            }),
            Self::GetCalibration => proto::Op::Feeder(proto::FeederOp::GetCalibration),
            Self::PutCalibration { base, calibration } => {
                proto::Op::Feeder(proto::FeederOp::PutCalibration {
                    base: base.0,
                    calibration: (*calibration).into(),
                })
            }
            Self::ReadBank1 { offset } => proto::Op::ReadBank1 { offset: *offset },
            Self::GetPlans => proto::Op::Feeder(proto::FeederOp::GetPlans),
            Self::PutPlans { base, plans } => proto::Op::Feeder(proto::FeederOp::PutPlans {
                base: base.0,
                plans: plans.map(|plan| plan.map(Into::into)),
            }),
            Self::ResetDesiccant => proto::Op::Feeder(proto::FeederOp::ResetDesiccant),
            Self::GetCall => proto::Op::Feeder(proto::FeederOp::GetCall),
            Self::StartCall { bytes, crc } => proto::Op::Feeder(proto::FeederOp::StartCall {
                bytes: *bytes,
                crc: *crc,
            }),
            Self::CallChunk { offset, data } => proto::Op::Feeder(proto::FeederOp::CallChunk {
                offset: *offset,
                data,
            }),
            Self::DeleteCall => proto::Op::Feeder(proto::FeederOp::DeleteCall),
            Self::PlayCall => proto::Op::Feeder(proto::FeederOp::PlayCall),
            // A time past the feeder's u32 seconds is no meal it holds: it
            // is refused `Invalid`.
            Self::SkipMeal { slot, due } => proto::Op::Feeder(proto::FeederOp::SkipMeal {
                slot: *slot,
                due_s: u32::try_from(due.as_second()).unwrap_or(u32::MAX),
            }),
            Self::UnskipMeal { slot, due } => proto::Op::Feeder(proto::FeederOp::UnskipMeal {
                slot: *slot,
                due_s: u32::try_from(due.as_second()).unwrap_or(u32::MAX),
            }),
            Self::GetAlerts => proto::Op::GetAlerts,
            Self::DeleteCallIf { bytes, crc } => proto::Op::Feeder(proto::FeederOp::DeleteCallIf {
                bytes: *bytes,
                crc: *crc,
            }),
        })
    }
}

impl TryFrom<FeederSettingsPatch> for proto::SettingsChanges {
    type Error = NoWireForm;

    fn try_from(patch: FeederSettingsPatch) -> Result<Self, NoWireForm> {
        let FeederSettingsPatch {
            lights,
            sound,
            volume,
            lights_window,
            sound_window,
            desiccant_days,
            meal_reminder,
        } = patch;
        Ok(Self {
            lights,
            sound,
            volume,
            lights_window: lights_window.map(TryInto::try_into).transpose()?,
            sound_window: sound_window.map(TryInto::try_into).transpose()?,
            desiccant_days,
            meal_reminder: meal_reminder.map(TryInto::try_into).transpose()?,
        })
    }
}

impl From<Tune> for proto::Tune {
    fn from(tune: Tune) -> Self {
        match tune {
            Tune::Chime => Self::Chime,
            Tune::Meal => Self::Meal,
            Tune::Coin => Self::Coin,
            Tune::Alert => Self::Alert,
            Tune::Test => Self::Test,
            Tune::Twinkle => Self::Twinkle,
        }
    }
}

impl From<TimeSync> for proto::TimeSync {
    fn from(sync: TimeSync) -> Self {
        let TimeSync {
            unix_ms,
            offset_s,
            next_change_s,
            next_offset_s,
        } = sync;
        Self {
            unix_ms,
            offset_s,
            next_change_s,
            next_offset_s,
        }
    }
}

impl From<Hello> for proto::Hello {
    fn from(hello: Hello) -> Self {
        let Hello { major } = hello;
        Self { major }
    }
}

impl From<proto::OpenRefusal> for OpenRefusal {
    fn from(refusal: proto::OpenRefusal) -> Self {
        match refusal {
            proto::OpenRefusal::Unknown => Self::Unknown,
            proto::OpenRefusal::Version { supported } => Self::Version { supported },
            proto::OpenRefusal::Full => Self::Full,
        }
    }
}

impl From<proto::Joined> for Joined {
    fn from(joined: proto::Joined) -> Self {
        match joined {
            proto::Joined::Enrolled { role, model } => Self::Enrolled {
                role: role.into(),
                model,
            },
            proto::Joined::Refused(refusal) => Self::Refused(refusal.into()),
        }
    }
}

/// A setup reply a client acts on; `None` for a step of the code proof,
/// which the client's session answers itself.
pub fn setup_reply(reply: proto_setup::Reply) -> Option<setup::Reply> {
    Some(match reply {
        proto_setup::Reply::EnterCode { seconds } => setup::Reply::EnterCode { seconds },
        proto_setup::Reply::Round { .. } => return None,
        proto_setup::Reply::Done { enrolled, wifi } => setup::Reply::Done {
            enrolled: match enrolled {
                proto_setup::Enrolled::New => setup::Enrolled::New,
                proto_setup::Enrolled::Already => setup::Enrolled::Already,
            },
            wifi: match wifi {
                proto_setup::WifiSaved::Saved => setup::WifiSaved::Saved,
                proto_setup::WifiSaved::Unchanged => setup::WifiSaved::Unchanged,
            },
        },
        proto_setup::Reply::Refused(refusal) => setup::Reply::Refused(match refusal {
            proto_setup::Refusal::Busy => setup::Refusal::Busy,
            proto_setup::Refusal::Unconfirmed => setup::Refusal::Unconfirmed,
            proto_setup::Refusal::WrongCode => setup::Refusal::WrongCode,
            proto_setup::Refusal::Full => setup::Refusal::Full,
            proto_setup::Refusal::NotWritable => setup::Refusal::NotWritable,
            proto_setup::Refusal::Invalid => setup::Refusal::Invalid,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::proto;

    /// An enum value a newer build added, inside an alert or an event,
    /// reads as unknown there, and the rest of the entry reads on: a failed
    /// meal with an outcome and reason this build can't name is still a
    /// failed meal, urgent, and its event still a feed's.
    #[test]
    fn a_newer_value_inside_an_alert_or_event_reads_as_unknown() {
        use crate::{
            Alert, AlertLevel, AlertsView, EventBody, FeedOutcome, FeedReason, FeederEvent,
        };

        let mut entries = [None; proto::ALERTS_MAX];
        entries[0] = Some(proto::OpenAlert {
            at: core::num::NonZeroU32::new(1_800_000_000),
            alert: proto::Alert::MealFailed {
                feed: 7,
                plan: Some(2),
                requested: 3,
                completed: 1,
                outcome: proto::Lenient::Unknown(99),
                reason: proto::Lenient::Unknown(98),
            },
        });
        let sent = proto::AlertsView {
            epoch: 1,
            generation: 2,
            more: false,
            settling: false,
            alerts: proto::AlertTable {
                entries,
                skipped: 0,
            },
        };
        let mut bytes = [0; 512];
        let len = wellfed_proto::encode(&sent, &mut bytes).unwrap();
        let view =
            AlertsView::from(wellfed_proto::decode::<proto::AlertsView>(&bytes[..len]).unwrap());

        assert_eq!(view.unread, 0);
        let [open] = view.alerts.as_slice() else {
            panic!("one alert: {view:?}");
        };
        assert_eq!(
            open.alert,
            Alert::MealFailed {
                feed: 7,
                plan: Some(2),
                requested: 3,
                completed: 1,
                outcome: FeedOutcome::Unknown,
                reason: FeedReason::Unknown,
            }
        );
        assert_eq!(open.alert.level(), AlertLevel::Urgent);

        let fed = proto::Event {
            seq: 4,
            at: None,
            body: proto::EventBody::Fed {
                feed: 7,
                requested: 3,
                completed: 1,
                outcome: proto::Lenient::Unknown(99),
                reason: proto::Lenient::Unknown(98),
                hopper: proto::Lenient::Unknown(97),
                by: proto::Source::Schedule(2),
            },
        };
        let raw = proto::RawEvent::encode(&fed).unwrap();
        let crate::Pushed::Event(event) = super::raw_event(1, raw) else {
            panic!("an event this build reads");
        };
        let EventBody::Feeder(FeederEvent::Fed {
            outcome, reason, ..
        }) = event.body
        else {
            panic!("a feed's event: {event:?}");
        };
        assert_eq!(
            (outcome, reason),
            (FeedOutcome::Unknown, FeedReason::Unknown)
        );
    }

    /// Every op names the module its wire form goes to, and one that has
    /// no wire form (a value only a newer build names) still names its own.
    #[test]
    fn an_op_s_kind_is_its_wire_form_s() {
        use crate::{
            Calibration, ClientRole, FeederSettingsPatch, Generation, Op, PLANS, StatsSection,
            TimeChanges, TimeSync, Tune, Window,
        };

        let at = jiff::Timestamp::from_second(1_800_000_000).unwrap();
        let ops = [
            Op::Status,
            Op::GetSettings,
            Op::PutSettings {
                base: Generation(1),
                changes: FeederSettingsPatch::default(),
            },
            Op::PutWifi {
                base: Generation(1),
                wifi: None,
            },
            Op::SetLock { locked: true },
            Op::Play { tune: Tune::Chime },
            Op::TimeSync(TimeSync {
                unix_ms: 1,
                offset_s: 0,
                next_change_s: 0,
                next_offset_s: 0,
            }),
            Op::CreateInvite {
                role: ClientRole::Member,
            },
            Op::CreateBoundInvite {
                role: ClientRole::Member,
                key: [1; 32],
            },
            Op::ListClients,
            Op::RevokeClient { key: [1; 32] },
            Op::Feed { portions: 1 },
            Op::Reboot,
            Op::CancelFeed { feed: 1 },
            Op::OfferUpdate {
                manifest: vec![1],
                downgrade: None,
            },
            Op::UpdateChunk {
                offset: 0,
                data: vec![1],
            },
            Op::UpdateState,
            Op::CancelUpdate,
            Op::EnrollUpdateKey {
                key: [1; 32],
                label: "key".into(),
            },
            Op::UpdateKeyStatement { statement: vec![1] },
            Op::GetTime,
            Op::PutTime {
                base: Generation(1),
                changes: TimeChanges::default(),
            },
            Op::TailLog { after: None },
            Op::Stats {
                section: StatsSection::Counters,
            },
            Op::Stats {
                section: StatsSection::Food,
            },
            Op::FeedTrace { feed: 1, offset: 0 },
            Op::GetCalibration,
            Op::PutCalibration {
                base: Generation(1),
                calibration: Calibration {
                    portion_mg: None,
                    bowl: crate::Bowl::Single,
                },
            },
            Op::ReadBank1 { offset: 0 },
            Op::GetPlans,
            Op::PutPlans {
                base: Generation(1),
                plans: [None; PLANS],
            },
            Op::ResetDesiccant,
            Op::GetCall,
            Op::StartCall { bytes: 1, crc: 1 },
            Op::CallChunk {
                offset: 0,
                data: vec![1],
            },
            Op::DeleteCall,
            Op::PlayCall,
            Op::SkipMeal { slot: 0, due: at },
            Op::UnskipMeal { slot: 0, due: at },
            Op::GetAlerts,
            Op::DeleteCallIf { bytes: 1, crc: 1 },
        ];
        for op in &ops {
            // A statistics section names its own kind, inside the shared op.
            let wire = match (op, op.to_proto().unwrap()) {
                (Op::Stats { section }, _) => section.kind(),
                (_, proto::Op::Feeder(_)) => Some(DeviceKind::Feeder),
                _ => None,
            };
            assert_eq!(op.kind(), wire, "{op:?}");
        }

        let unsendable = Op::PutSettings {
            base: Generation(1),
            changes: FeederSettingsPatch {
                lights_window: Some(Window::Unknown),
                ..FeederSettingsPatch::default()
            },
        };
        assert!(unsendable.to_proto().is_err());
        assert_eq!(unsendable.kind(), Some(DeviceKind::Feeder));
    }

    use super::*;
    use crate::{AlertKey, AlertKind, AlertLevel};

    /// The client's gate is the device's matrix: one op of each class, by
    /// owner, member, server, update signer and a role a newer build named.
    #[test]
    fn a_role_may_send_what_the_device_allows_it() {
        let roles = [
            ClientRole::Owner,
            ClientRole::Member,
            ClientRole::Server,
            ClientRole::UpdateSigner,
            ClientRole::Unknown,
        ];
        let calibrate = Op::PutCalibration {
            base: Generation(0),
            calibration: crate::Calibration {
                portion_mg: Some(9_000),
                bowl: crate::Bowl::Single,
            },
        };
        let invite = |role| Op::CreateInvite { role };
        let rows = [
            (Op::Feed { portions: 1 }, [true, true, true, false, false]),
            (calibrate, [true, true, false, false, false]),
            (Op::Reboot, [true, false, false, false, false]),
            // The matrix lets all three ask for an invite; the role it
            // grants is pairing's second check.
            (
                invite(ClientRole::Member),
                [true, true, false, false, false],
            ),
            (
                invite(ClientRole::Owner),
                [true, false, false, false, false],
            ),
            (
                invite(ClientRole::UpdateSigner),
                [false, false, false, false, false],
            ),
        ];

        for (op, may) in rows {
            for (role, may) in roles.into_iter().zip(may) {
                assert_eq!(op.allowed(role), may, "{role:?} {op:?}");
            }
        }
    }

    /// One alert of every kind, as the feeder's rules hold them.
    fn every_alert() -> Vec<proto::OpenAlert> {
        let at = core::num::NonZeroU32::new(1_800_000_000);
        let alerts = [
            proto::Alert::MealFailed {
                feed: 7,
                plan: Some(1),
                requested: 2,
                completed: 1,
                outcome: proto::FeedOutcome::Failed.into(),
                reason: proto::FeedReason::Jammed.into(),
            },
            proto::Alert::MealUncertain {
                feed: 8,
                plan: None,
                requested: 2,
                completed: 0,
                reason: proto::FeedReason::Interrupted.into(),
            },
            proto::Alert::MealMissed {
                plan: 2,
                due: 1_800_000_100,
                reason: proto::MissedReason::Late,
            },
            proto::Alert::MealNoRecord {
                plan: 3,
                due: 1_800_000_200,
            },
            proto::Alert::PowerWasOut,
            proto::Alert::UpdateFailed {
                stop: proto::UpdateStop::Hash.into(),
            },
            proto::Alert::RolledBack,
            proto::Alert::ClientJoined {
                client: CLIENT,
                role: proto::Role::Member.into(),
                path: proto::Path::Invite.into(),
            },
            proto::Alert::ClientRevoked {
                client: CLIENT,
                by: proto::Source::Panel,
            },
            proto::Alert::RepeatedRestarts,
            proto::Alert::NoClock,
            proto::Alert::CannotFeed {
                reason: proto::Blocked::Fault.into(),
            },
            proto::Alert::FoodLow,
            proto::Alert::FoodLikelyOut { confirmed: 5 },
            proto::Alert::DesiccantDue { days: -2 },
            proto::Alert::BatteryBelowFloor,
            proto::Alert::PlansStuck,
        ];
        alerts
            .into_iter()
            .map(|alert| proto::OpenAlert { at, alert })
            .collect()
    }

    fn key_of(key: proto::AlertKey) -> AlertKey {
        let at = |at: Option<core::num::NonZeroU32>| at.map(|at| seconds(at.get()));
        match key {
            proto::AlertKey::Feed { feed } => AlertKey::Feed { feed },
            proto::AlertKey::Meal { plan, due } => AlertKey::Meal {
                plan,
                due: seconds(due),
            },
            proto::AlertKey::Client {
                kind,
                client,
                at: when,
            } => AlertKey::Client {
                kind: AlertKind::of_wire(kind),
                client: Hex(client),
                at: at(when),
            },
            proto::AlertKey::Timed { kind, at: when } => AlertKey::Timed {
                kind: AlertKind::of_wire(kind),
                at: at(when),
            },
            proto::AlertKey::Kind(kind) => AlertKey::Kind {
                kind: AlertKind::of_wire(kind),
            },
        }
    }

    /// A client keys, ranks and sorts an alert as the feeder's rules do, for
    /// every kind: the app's notifications depend on it.
    #[test]
    fn an_alert_has_the_feeders_key_kind_and_level() {
        let every = every_alert();
        assert_eq!(every.len(), AlertKind::ALL.len());
        for (wire, kind) in every.iter().zip(AlertKind::ALL) {
            let open = OpenAlert::from(wire);
            assert_eq!(open.alert.kind(), kind, "ALL is in the enum's order");
            assert_eq!(AlertKind::of_wire(wire.alert.kind()), kind);
            assert_eq!(AlertLevel::of_wire(wire.alert.level()), open.alert.level());
            assert_eq!(wire.alert.kind().is_condition(), kind.is_condition());
            assert_eq!(key_of(wire.key()), open.key(), "{kind:?}");
        }
    }

    /// The feeder's "no generation" (its settings records apart, read-only
    /// until it repairs them) decodes to none; JSON keeps the number, so
    /// `/api/v1` and its older readers see the field unchanged.
    #[test]
    fn settings_with_no_generation_decode_to_none() {
        let wire = proto::Settings {
            generation: proto::NO_SETTINGS_GENERATION,
            lights: true,
            sound: true,
            volume: 50,
            lights_window: proto::Window::Always,
            sound_window: proto::Window::Always,
            desiccant_days: 14,
            desiccant_reset: None,
            meal_reminder: proto::MealReminder::Off,
            writable: false,
            ext: proto::Tail::EMPTY,
        };
        let settings = FeederSettings::from(wire);
        assert_eq!((settings.generation, settings.writable), (None, false));

        let json = serde_json::to_value(settings).unwrap();
        assert_eq!(json["generation"], u32::MAX);
        let back: FeederSettings = serde_json::from_value(json).unwrap();
        assert_eq!(back, settings);

        let held = FeederSettings::from(proto::Settings {
            generation: 7,
            ..wire
        });
        assert_eq!(held.generation, Some(Generation(7)));
        assert_eq!(serde_json::to_value(held).unwrap()["generation"], 7);
    }

    const CLIENT: [u8; 4] = [0xde, 0xad, 0xbe, 0xef];

    /// Every body, so a round trip covers each variant's fields.
    fn bodies() -> Vec<proto::EventBody> {
        let by = proto::Source::Client(CLIENT);
        vec![
            proto::EventBody::Boot {
                bank: 1,
                reset_flags: 0x40,
            },
            proto::EventBody::Lock { locked: true, by },
            proto::EventBody::Settings {
                generation: 7,
                by: proto::Source::Panel,
            },
            proto::EventBody::Wifi {
                generation: 2,
                by: proto::Source::Feeder,
            },
            proto::EventBody::Enrolled {
                client: CLIENT,
                role: proto::Role::UpdateSigner.into(),
                path: proto::Path::Recovery.into(),
            },
            proto::EventBody::Revoked { client: CLIENT, by },
            proto::EventBody::ClockSync {
                by,
                jump_ms: Some(-5),
            },
            proto::EventBody::Lost { count: 3 },
            proto::EventBody::Fed {
                feed: 9,
                requested: 2,
                completed: 1,
                outcome: proto::FeedOutcome::Uncertain.into(),
                reason: proto::FeedReason::FeedbackStream.into(),
                hopper: proto::FoodLevel::Low.into(),
                by,
            },
            proto::EventBody::FeedingBlocked {
                reason: Some(proto::Blocked::Fault.into()),
            },
            proto::EventBody::FeedingBlocked { reason: None },
            proto::EventBody::MealSoon {
                plan: 3,
                due: 1_793_200_000,
                minute: 450,
                portions: 2,
            },
            proto::EventBody::Food {
                level: proto::FoodLevel::Present.into(),
            },
            proto::EventBody::UpdateStop {
                stop: proto::UpdateStop::Hash.into(),
            },
            proto::EventBody::UpdateAtBoot {
                candidate: proto::UpdateCandidate::Pending.into(),
            },
            proto::EventBody::Restarting,
            proto::EventBody::UpdateConfirmed,
        ]
    }

    #[test]
    fn a_stored_body_decodes_to_the_body_received() {
        for body in bodies() {
            let model = EventBody::from(body);
            assert_eq!(proto::EventBody::try_from(model), Ok(body));
        }
    }

    /// `wellfed events --json` prints each body as this JSON.
    #[test]
    fn a_meal_reminder_reads_as_json() {
        let body = EventBody::from(proto::EventBody::MealSoon {
            plan: 3,
            due: 1_793_200_000,
            minute: 450,
            portions: 2,
        });
        assert_eq!(
            serde_json::to_value(body).unwrap(),
            serde_json::json!({
                "kind": "meal_soon",
                "plan": 3,
                "due": "2026-10-28T15:06:40Z",
                "minute": 450,
                "portions": 2,
            })
        );
        let reminder = serde_json::to_value(MealReminder::Before { minutes: 10 }).unwrap();
        assert_eq!(
            reminder,
            serde_json::json!({ "mode": "before", "minutes": 10 })
        );
    }

    #[test]
    fn a_stored_sample_value_decodes_to_the_value_received() {
        for metric in [
            proto::Metric::Power(proto::Power {
                adapter: false,
                battery_mv: Some(4100),
            }),
            proto::Metric::Rssi(-61),
            proto::Metric::Heap {
                free: 125_088,
                min: 119_000,
            },
            proto::Metric::Loop {
                passes: 3_000,
                max_us: 27_400,
                p99_us: 1_100,
            },
            proto::Metric::Gateway {
                round_trip_ms: Some(4),
            },
            proto::Metric::Wifi {
                channel: Some(11),
                ap_changes: 1,
            },
        ] {
            assert_eq!(
                proto::Metric::try_from(SampleValue::from(metric)),
                Ok(metric)
            );
        }
    }

    #[test]
    fn a_body_reads_as_the_http_api_spells_it() {
        let body = EventBody::from(proto::EventBody::Enrolled {
            client: CLIENT,
            role: proto::Role::Owner.into(),
            path: proto::Path::Invite.into(),
        });
        assert_eq!(
            serde_json::to_value(body).unwrap(),
            serde_json::json!({
                "kind": "enrolled",
                "client": "deadbeef",
                "role": "owner",
                "path": "invite",
            })
        );
    }

    #[test]
    fn a_clock_past_jiffs_range_reads_as_unknown() {
        let status = proto::Status {
            build: "b",
            bank: 0,
            uptime_ms: 0,
            boot_count: 0,
            reset_flags: 0,
            faults: 0,
            rssi: 0,
            wifi: proto::WifiSource::Build.into(),
            link_losses: 0,
            dns_losses: 0,
            clock_ms: Some(u64::MAX),
            power: None,
            wifi_generation: 0,
            last_event: None,
            sessions: 1,
            alerts_generation: 0,
            device: proto::DeviceStatus::Feeder(proto::FeederStatus {
                locked: None,
                settings_generation: 0,
                desiccant: proto::Desiccant::Unknown,
                food: proto::FoodLevel::Unread.into(),
                feed_fault: None,
                blocked: None,
                supply: proto::Supply {
                    adapter: None,
                    battery: proto::Battery::Unread,
                    below_floor: None,
                },
                ext: proto::Tail::EMPTY,
            }),
            ext: proto::Tail::EMPTY,
        };
        assert_eq!(Status::from(status).clock, None);
    }

    #[test]
    fn the_desiccant_count_says_when_it_is_due() {
        let cases = [
            (proto::Desiccant::Unset, "not started", None),
            (proto::Desiccant::Unknown, "unknown (no clock)", None),
            (
                proto::Desiccant::Remaining { days: 14 },
                "14 days left",
                Some(false),
            ),
            (
                proto::Desiccant::Remaining { days: 1 },
                "1 day left",
                Some(false),
            ),
            (
                proto::Desiccant::Remaining { days: 0 },
                "due today",
                Some(true),
            ),
            (
                proto::Desiccant::Remaining { days: -1 },
                "due (1 day over)",
                Some(true),
            ),
            (
                proto::Desiccant::Remaining { days: -52 },
                "due (52 days over)",
                Some(true),
            ),
        ];
        for (wire, text, due) in cases {
            let desiccant = Desiccant::from(wire);
            assert_eq!(desiccant.to_string(), text);
            let shown = match desiccant {
                Desiccant::Remaining { due, .. } => Some(due),
                Desiccant::Unset | Desiccant::Unknown => None,
            };
            assert_eq!(shown, due, "{text}");
        }
        assert_eq!(
            serde_json::to_value(Desiccant::from(proto::Desiccant::Remaining { days: -2 }))
                .unwrap(),
            serde_json::json!({ "state": "remaining", "days": -2, "due": true })
        );
        assert_eq!(calendar_day(20_389), Some(jiff::civil::date(2025, 10, 28)));
        assert_eq!(calendar_day(-1), Some(jiff::civil::date(1969, 12, 31)));
        assert_eq!(calendar_day(i32::MAX), None);
    }

    #[test]
    fn a_client_list_keeps_only_the_enrolled_slots_in_order() {
        let mut clients = [None; proto::CLIENTS_MAX];
        let client = |label, you| proto::Client {
            key: [1; 32],
            label,
            role: proto::Role::Member,
            you,
        };
        clients[1] = Some(client("phone", true));
        clients[4] = Some(client("server", false));

        let Answer::Clients(listed) =
            Answer::from(proto::Outcome::Clients(proto::Clients { clients }))
        else {
            panic!("a client list");
        };
        let labels: Vec<_> = listed.iter().map(|c| (c.label.as_str(), c.you)).collect();
        assert_eq!(labels, [("phone", true), ("server", false)]);
    }

    #[test]
    fn an_update_view_keeps_the_wanted_offsets_in_order_and_reads_as_hex() {
        let mut wanted = [None; proto::WANTED_MAX];
        wanted[0] = Some(4096);
        wanted[1] = Some(4608);
        let view = proto::UpdateView {
            phase: proto::UpdatePhase::Receiving,
            bank: 0,
            min_version: 3,
            digest: Some([0xab; 32]),
            version: 4,
            image_len: 8192,
            written: 4096,
            wanted,
            stopped: None,
            bank0: proto::Bank0Check::Blocked {
                reason: proto::Bank0Blocked::Unrecorded,
            },
        };

        let Answer::Update(view) = Answer::from(proto::Outcome::Update(view)) else {
            panic!("an update view");
        };
        assert_eq!(view.wanted, [4096, 4608]);
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["phase"], "receiving");
        assert_eq!(json["digest"], "ab".repeat(32));
        assert_eq!(
            json["bank0"],
            serde_json::json!({"state": "blocked", "reason": "unrecorded"})
        );
    }

    #[test]
    fn a_proof_round_is_no_reply_a_client_acts_on() {
        let round = proto_setup::Reply::Round {
            open: None,
            commit: Some([2; 32]),
        };
        assert_eq!(setup_reply(round), None);
        assert_eq!(
            setup_reply(proto_setup::Reply::Refused(proto_setup::Refusal::WrongCode)),
            Some(setup::Reply::Refused(setup::Refusal::WrongCode))
        );
    }

    #[test]
    fn an_op_borrows_its_credentials_for_the_wire() {
        let op = Op::PutWifi {
            base: Generation(4),
            wifi: Some(crate::Wifi {
                ssid: b"home".to_vec(),
                psk: "password1".to_owned(),
            }),
        };
        assert_eq!(
            op.to_proto(),
            Ok(proto::Op::PutWifi {
                base: 4,
                wifi: Some(proto::Wifi {
                    ssid: b"home",
                    psk: "password1",
                }),
            })
        );
        assert!(!format!("{op:?}").contains("password1"));
    }
}
