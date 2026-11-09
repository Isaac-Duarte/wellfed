//! The feeder's durable history and its live samples, as a session
//! receives them.
//!
//! Event numbers count within an epoch: the feeder's ring draws one when it
//! is created, and a ring re-created by a storage wipe counts from 1 again
//! under a new one. Each epoch is its own history.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{Battery, Blocked, DeviceKind, Generation, Hex, UpdateCandidate, UpdateStop};

/// What the feeder pushes to a subscribed session, or to one that asked
/// for its log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pushed {
    Event(Event),
    /// An event whose body this build does not know: a newer feeder's.
    Unknown(UnknownEvent),
    Gap(Gap),
    Sample(Sample),
    Log(LogLines),
}

/// Record lines the feeder logged, as `TailLog` streams them. The text is
/// diagnostics, not a contract: any build may change it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLines {
    /// The last line's number this boot: the cursor to resume after.
    pub seq: u32,
    /// Lines after the previous cursor the feeder overwrote before sending
    /// them.
    pub dropped: u32,
    /// The lines, without their endings.
    pub lines: Vec<String>,
}

/// One event the feeder recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// The history the event belongs to.
    pub epoch: u32,
    /// The feeder's sequence number: monotonic across its reboots, within
    /// the epoch.
    pub seq: u64,
    /// The feeder's clock when it happened; `None` while that was invalid.
    pub at: Option<Timestamp>,
    pub body: EventBody,
}

/// An event the feeder recorded whose body this build cannot read, since a
/// newer build added it. It still counts in the history: a cursor moves
/// past it, and a host that keeps the body can read it once it is updated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownEvent {
    pub epoch: u32,
    pub seq: u64,
    pub at: Option<Timestamp>,
    /// The body as the feeder's ring keeps it, zero-padded.
    pub raw: Hex<16>,
}

/// Events `from..=to` of `epoch` left the feeder's ring before this session
/// read them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gap {
    pub epoch: u32,
    pub from: u64,
    pub to: u64,
}

/// What an event says: one of the shared core's, which every kind of
/// device records, or one of its own kind's. The JSON is flat, tagged by
/// the event's `kind` alone, as the history stores it; [`Self::device`]
/// names the kind whose module records it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum EventBody {
    Core(CoreEvent),
    Feeder(FeederEvent),
}

/// An event every kind of device records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CoreEvent {
    Boot {
        bank: u8,
        reset_flags: u32,
    },
    Wifi {
        generation: Generation,
        by: Source,
    },
    Enrolled {
        /// The new client's fingerprint (its key's first 4 bytes).
        #[schema(value_type = String)]
        client: Hex<4>,
        role: ClientRole,
        path: EnrollPath,
    },
    Revoked {
        #[schema(value_type = String)]
        client: Hex<4>,
        by: Source,
    },
    ClockSync {
        by: Source,
        /// How far the clock moved, when it was valid before.
        jump_ms: Option<i32>,
    },
    /// Events the feeder could not record (its queue overflowed).
    Lost {
        count: u16,
    },
    /// An install stopped before activation, a cancel among them.
    UpdateStop {
        stop: UpdateStop,
    },
    /// What the update store held as this boot came up.
    UpdateAtBoot {
        candidate: UpdateCandidate,
    },
    /// A restart someone asked for is on its way.
    Restarting,
    /// The build this boot activated confirmed itself, with no restart.
    UpdateConfirmed,
}

/// An event only a feeder records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FeederEvent {
    Lock {
        locked: bool,
        by: Source,
    },
    Settings {
        generation: Generation,
        by: Source,
    },
    /// A feed ended.
    Fed {
        /// The feeder's number for the feed.
        feed: u32,
        requested: u8,
        /// The portions the feedback sensor confirmed.
        completed: u8,
        outcome: FeedOutcome,
        reason: FeedReason,
        /// What the food beam read during the feed. A feed runs whatever
        /// it reads, so low food is recorded here, not refused.
        hopper: FoodLevel,
        by: Source,
    },
    /// The lasting reason the feeder cannot feed changed: the new reason,
    /// or none once it can feed again.
    FeedingBlocked {
        reason: Option<Blocked>,
    },
    /// A client replaced the feeding plans.
    Plans {
        generation: Generation,
        by: Source,
    },
    /// A plan's occurrence did not run, and never will.
    Missed {
        /// The plan's slot.
        plan: u8,
        /// When it was due.
        due: Timestamp,
        reason: MissedReason,
    },
    /// A plan's occurrence is expected: sent the meal reminder's lead
    /// ahead of it. A feeder restart inside the lead may send it again,
    /// with the same `plan` and `due`. It is never withdrawn: the meal may
    /// still be missed, or the plan disabled, deleted or moved, so the
    /// `fed` or `missed` event for that `plan` and `due` (or the plans) say
    /// what happened.
    MealSoon {
        /// The plan's slot.
        plan: u8,
        /// When it is due.
        due: Timestamp,
        /// The minute of the feeder's local day it is due at (0-1439).
        minute: u16,
        /// The portions it will ask for.
        portions: u8,
    },
    /// The adapter came or went, or the pack crossed its floor (at most
    /// one a minute, as the supply stood at its end).
    Supply {
        adapter: Option<bool>,
        battery: Battery,
        /// Whether the feeding owner judged the pack below its floor.
        below_floor: Option<bool>,
    },
    /// Dispense was pressed on an unlocked panel while a feed could not
    /// start, during a feed among them: it started nothing and was not
    /// queued.
    DispenseRefused {
        reason: Blocked,
    },
    /// The food beam's level changed (at most once a feed or a reading at
    /// rest). A boot's first reading counts as one only if a feed took it.
    Food {
        level: FoodLevel,
    },
    /// A client skipped a plan's meal still to come: it does not feed
    /// unless the skip is undone before it is due.
    Skipped {
        /// The plan's slot.
        plan: u8,
        /// When the meal is due.
        due: Timestamp,
        by: Source,
    },
    /// A client undid the skip of a plan's meal: it feeds as planned.
    Unskipped {
        /// The plan's slot.
        plan: u8,
        /// When the meal is due.
        due: Timestamp,
        by: Source,
    },
}

/// What the food beam across the hopper reads. One reading with food shows
/// food at once; the hopper reads low after a second without (stock
/// counts 20 readings; the second assumes its poll is 50 ms).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FoodLevel {
    /// Not read yet: the beam reads soon after a boot, on the adapter, or
    /// during the first feed.
    Unread,
    Present,
    /// No food in front of the beam, which sits above the rotor, so about five
    /// portions remain (at any time during a feed, for a `Fed` event).
    Low,
    /// A value a newer feeder build added, which this build cannot name.
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FeedOutcome {
    Done,
    Cancelled,
    /// It never started: the feeder could not get ready in time.
    Expired,
    Failed,
    /// The feeder stopped without recording a result: anything up to
    /// `requested` may have been dispensed. Never retried automatically.
    /// When a power cut lost the reservation itself, `requested` is the
    /// feeder's cap on one feed, not what was asked.
    Uncertain,
    /// A value a newer feeder build added, which this build cannot name.
    #[serde(other)]
    Unknown,
}

/// Why a feed ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FeedReason {
    Done,
    Cancelled,
    /// The feeder's clock jumped.
    Clock,
    /// The feeder could not watch the motor often enough.
    ObservationGap,
    /// The whole feed ran past its limit.
    Timeout,
    /// No rotor mark for too long: the mark sensor lost, or a rotor stalled
    /// with no current to show it.
    NoFeedback,
    Power,
    /// A sensor could not be read (the ADC or the beams).
    Sensor,
    /// The outlet beam read blocked: mid-feed, the feed ended with the
    /// portions delivered so far, never resumed; at the start, no motion.
    OutletBlocked,
    /// One current reading over the ceiling, or the limit held while the
    /// motor is stopped (a pause): stopped at once.
    Overcurrent,
    /// The feedback sensor's edges could not be trusted.
    FeedbackStream,
    /// An output failed to apply or verify.
    Output,
    /// A jam three reversals did not free (or one that held while backing
    /// off). Never retried.
    Jammed,
    /// The mark sensor read a mark for longer than any mark lasts.
    MarkStuck,
    /// A reset (power cut, watchdog, fault) cut the feed short, and the
    /// next boot resolved it: always `Uncertain`.
    Interrupted,
    /// A value a newer feeder build added, which this build cannot name.
    #[serde(other)]
    Unknown,
}

impl FeedOutcome {
    pub const ALL: [Self; 5] = [
        Self::Done,
        Self::Cancelled,
        Self::Expired,
        Self::Failed,
        Self::Uncertain,
    ];

    /// The stored spelling, the same as the JSON one.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
            Self::Failed => "failed",
            Self::Uncertain => "uncertain",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .chain([Self::Unknown])
            .find(|outcome| outcome.code() == code)
    }
}

impl FeedReason {
    pub const ALL: [Self; 15] = [
        Self::Done,
        Self::Cancelled,
        Self::Clock,
        Self::ObservationGap,
        Self::Timeout,
        Self::NoFeedback,
        Self::Power,
        Self::Sensor,
        Self::OutletBlocked,
        Self::Overcurrent,
        Self::FeedbackStream,
        Self::Output,
        Self::Jammed,
        Self::MarkStuck,
        Self::Interrupted,
    ];

    /// The stored spelling, the same as the JSON one.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Cancelled => "cancelled",
            Self::Clock => "clock",
            Self::ObservationGap => "observation_gap",
            Self::Timeout => "timeout",
            Self::NoFeedback => "no_feedback",
            Self::Power => "power",
            Self::Sensor => "sensor",
            Self::OutletBlocked => "outlet_blocked",
            Self::Overcurrent => "overcurrent",
            Self::FeedbackStream => "feedback_stream",
            Self::Output => "output",
            Self::Jammed => "jammed",
            Self::MarkStuck => "mark_stuck",
            Self::Interrupted => "interrupted",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .chain([Self::Unknown])
            .find(|reason| reason.code() == code)
    }
}

impl EventBody {
    pub const fn kind(&self) -> EventKind {
        match self {
            Self::Core(event) => match event {
                CoreEvent::Boot { .. } => EventKind::Boot,
                CoreEvent::Wifi { .. } => EventKind::Wifi,
                CoreEvent::Enrolled { .. } => EventKind::Enrolled,
                CoreEvent::Revoked { .. } => EventKind::Revoked,
                CoreEvent::ClockSync { .. } => EventKind::ClockSync,
                CoreEvent::Lost { .. } => EventKind::Lost,
                CoreEvent::UpdateStop { .. } => EventKind::UpdateStop,
                CoreEvent::UpdateAtBoot { .. } => EventKind::UpdateAtBoot,
                CoreEvent::Restarting => EventKind::Restarting,
                CoreEvent::UpdateConfirmed => EventKind::UpdateConfirmed,
            },
            Self::Feeder(event) => match event {
                FeederEvent::Lock { .. } => EventKind::Lock,
                FeederEvent::Settings { .. } => EventKind::Settings,
                FeederEvent::Fed { .. } => EventKind::Fed,
                FeederEvent::FeedingBlocked { .. } => EventKind::FeedingBlocked,
                FeederEvent::Plans { .. } => EventKind::Plans,
                FeederEvent::Missed { .. } => EventKind::Missed,
                FeederEvent::MealSoon { .. } => EventKind::MealSoon,
                FeederEvent::Supply { .. } => EventKind::Supply,
                FeederEvent::DispenseRefused { .. } => EventKind::DispenseRefused,
                FeederEvent::Food { .. } => EventKind::Food,
                FeederEvent::Skipped { .. } => EventKind::Skipped,
                FeederEvent::Unskipped { .. } => EventKind::Unskipped,
            },
        }
    }

    /// The kind of device whose module records it, `None` for the shared
    /// core's.
    pub const fn device(&self) -> Option<DeviceKind> {
        self.kind().device()
    }
}

impl From<CoreEvent> for EventBody {
    fn from(event: CoreEvent) -> Self {
        Self::Core(event)
    }
}

impl From<FeederEvent> for EventBody {
    fn from(event: FeederEvent) -> Self {
        Self::Feeder(event)
    }
}

/// An event's kind, for filtering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Boot,
    Lock,
    Settings,
    Wifi,
    Enrolled,
    Revoked,
    ClockSync,
    Lost,
    Fed,
    FeedingBlocked,
    Supply,
    Plans,
    Missed,
    DispenseRefused,
    MealSoon,
    Food,
    Skipped,
    Unskipped,
    UpdateStop,
    UpdateAtBoot,
    Restarting,
    UpdateConfirmed,
}

impl EventKind {
    pub const ALL: [Self; 22] = [
        Self::Boot,
        Self::Lock,
        Self::Settings,
        Self::Wifi,
        Self::Enrolled,
        Self::Revoked,
        Self::ClockSync,
        Self::Lost,
        Self::Fed,
        Self::FeedingBlocked,
        Self::Supply,
        Self::Plans,
        Self::Missed,
        Self::DispenseRefused,
        Self::MealSoon,
        Self::Food,
        Self::Skipped,
        Self::Unskipped,
        Self::UpdateStop,
        Self::UpdateAtBoot,
        Self::Restarting,
        Self::UpdateConfirmed,
    ];

    /// The stored spelling, the same as the JSON one.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Boot => "boot",
            Self::Lock => "lock",
            Self::Settings => "settings",
            Self::Wifi => "wifi",
            Self::Enrolled => "enrolled",
            Self::Revoked => "revoked",
            Self::ClockSync => "clock_sync",
            Self::Lost => "lost",
            Self::Fed => "fed",
            Self::FeedingBlocked => "feeding_blocked",
            Self::Supply => "supply",
            Self::Plans => "plans",
            Self::Missed => "missed",
            Self::DispenseRefused => "dispense_refused",
            Self::MealSoon => "meal_soon",
            Self::Food => "food",
            Self::Skipped => "skipped",
            Self::Unskipped => "unskipped",
            Self::UpdateStop => "update_stop",
            Self::UpdateAtBoot => "update_at_boot",
            Self::Restarting => "restarting",
            Self::UpdateConfirmed => "update_confirmed",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.code() == code)
    }

    /// The kind of device whose module records it, `None` for the shared
    /// core's: a filter for one kind's events reads only these.
    pub const fn device(self) -> Option<DeviceKind> {
        match self {
            Self::Boot
            | Self::Wifi
            | Self::Enrolled
            | Self::Revoked
            | Self::ClockSync
            | Self::Lost
            | Self::UpdateStop
            | Self::UpdateAtBoot
            | Self::Restarting
            | Self::UpdateConfirmed => None,
            Self::Lock
            | Self::Settings
            | Self::Fed
            | Self::FeedingBlocked
            | Self::Plans
            | Self::Missed
            | Self::MealSoon
            | Self::Supply
            | Self::DispenseRefused
            | Self::Food
            | Self::Skipped
            | Self::Unskipped => Some(DeviceKind::Feeder),
        }
    }
}

/// Who caused an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    Panel,
    Client {
        /// The client's fingerprint (its key's first 4 bytes).
        #[schema(value_type = String)]
        client: Hex<4>,
    },
    Feeder,
    /// A feeding plan.
    Schedule {
        /// The plan's slot.
        plan: u8,
    },
}

/// Why a plan's occurrence did not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MissedReason {
    /// It fell while the feeder was off or had no valid time, or the clock
    /// moved past it, more than 15 minutes ago.
    Late,
    /// It waited past 15 minutes for a feed the feeder could not start.
    Blocked { reason: Blocked },
    /// The feeder stopped after it took the occurrence and before the
    /// feed was reserved.
    Interrupted,
}

/// A role in the feeder's own client table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClientRole {
    Owner,
    Member,
    Server,
    UpdateSigner,
}

impl ClientRole {
    /// The stored spelling, the same as the JSON one.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Member => "member",
            Self::Server => "server",
            Self::UpdateSigner => "update_signer",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        [Self::Owner, Self::Member, Self::Server, Self::UpdateSigner]
            .into_iter()
            .find(|role| role.code() == code)
    }
}

/// How a client enrolled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnrollPath {
    Setup,
    Invite,
    Recovery,
    /// A session asked and a press on the feeder's panel confirmed: how an
    /// update signer's key joins.
    Press,
}

/// A live measurement, sent only to subscribers and never stored on the
/// feeder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sample {
    /// The feeder's uptime when it measured.
    pub uptime_ms: u32,
    pub value: SampleValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "metric", rename_all = "snake_case")]
pub enum SampleValue {
    Power {
        adapter: bool,
        battery_mv: Option<u32>,
    },
    Rssi {
        dbm: i32,
    },
    /// The heap's free bytes now, and the fewest since boot.
    Heap {
        free: u32,
        min: u32,
    },
    /// The loop's passes since the last loop sample: how many, the longest
    /// and the 99th percentile.
    Loop {
        passes: u32,
        max_us: u32,
        p99_us: u32,
    },
    /// The latest gateway round trip answered since the last gateway
    /// sample; `None` when none was.
    Gateway {
        round_trip_ms: Option<u32>,
    },
    /// The joined access point's channel, and how often a join landed on
    /// another one this boot.
    Wifi {
        channel: Option<u8>,
        ap_changes: u32,
    },
    /// The supply, judged.
    Supply {
        adapter: Option<bool>,
        battery: Battery,
        below_floor: Option<bool>,
    },
}

impl SampleValue {
    pub const fn metric(&self) -> MetricKind {
        match self {
            Self::Power { .. } => MetricKind::Power,
            Self::Rssi { .. } => MetricKind::Rssi,
            Self::Heap { .. } => MetricKind::Heap,
            Self::Loop { .. } => MetricKind::Loop,
            Self::Gateway { .. } => MetricKind::Gateway,
            Self::Wifi { .. } => MetricKind::Wifi,
            Self::Supply { .. } => MetricKind::Supply,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    Power,
    Rssi,
    Heap,
    Loop,
    Gateway,
    Wifi,
    Supply,
}

impl MetricKind {
    pub const ALL: [Self; 7] = [
        Self::Power,
        Self::Rssi,
        Self::Heap,
        Self::Loop,
        Self::Gateway,
        Self::Wifi,
        Self::Supply,
    ];

    /// The stored and query spelling, the same as the JSON one.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Power => "power",
            Self::Rssi => "rssi",
            Self::Heap => "heap",
            Self::Loop => "loop",
            Self::Gateway => "gateway",
            Self::Wifi => "wifi",
            Self::Supply => "supply",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|metric| metric.code() == code)
    }
}
