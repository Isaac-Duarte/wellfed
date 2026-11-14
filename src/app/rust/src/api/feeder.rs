//! What Dart asks a feeder and what comes back, beside the card and the feed
//! the session's stream carries. [`crate::requests`] turns each request into
//! the device API's ops, so Dart only ever names what the owner chose.
//!
//! Every write goes out once, under a fresh id: an answer that never came is
//! [`FeederReply::Unanswered`], which says the write may have run, and nothing
//! sends it again. [`FeederReply::NotSent`] means it never reached the
//! feeder, and says why.

pub use crate::api::model::{
    Bank0Blocked, Bank0Check, Blocked, Bowl, CallClip, ClientRole, Desiccant, FeedOutcome,
    FeedReason, FoodLevel, MealReminder, NtpResult, Phase, Stats, StatsSection, TimeSource,
    TraceEntry, TraceSummary, Tune, UpdatePhase, UpdateRefusal, Weekday, Window,
};
use flutter_rust_bridge::frb;

use crate::api::router::NotSentReason;

/// The time zones an owner picks from for the feeder, as IANA names, sorted.
/// Choosing one sends [`ZoneChange::Named`], whose rule is checked then.
#[frb(sync)]
pub fn time_zones() -> Vec<String> {
    wellfed_client::time::zones()
        .into_iter()
        .map(str::to_owned)
        .collect()
}

/// What the owner asks of a feeder.
pub enum FeederRequest {
    GetSettings,
    PutSettings {
        base: u32,
        changes: SettingsChanges,
    },
    /// Stored for the feeder's next join. The rules are checked here first.
    PutWifi {
        base: u32,
        ssid: String,
        passphrase: String,
    },
    ForgetWifi {
        base: u32,
    },
    SetLock {
        locked: bool,
    },
    Play {
        tune: Tune,
    },
    /// This phone's clock and zone.
    SyncClock,
    GetTime,
    PutTime {
        base: u32,
        zone: Option<ZoneChange>,
        ntp: Option<NtpChoice>,
    },
    CreateInvite {
        role: ClientRole,
    },
    ListClients,
    RevokeClient {
        key: Vec<u8>,
    },
    Reboot,
    ResetDesiccant,
    Stats {
        section: StatsSection,
    },
    /// Every page of one feed's trace.
    ReadTrace {
        feed: u32,
    },
    GetCalibration,
    PutCalibration {
        base: u32,
        portion: PortionChange,
        bowl: Bowl,
    },
    GetPlans,
    /// Skip one of today's meals, named as the card's `MealAction::Skip`
    /// names it.
    SkipMeal {
        slot: u8,
        due_ms: i64,
    },
    /// Undo a meal's skip, named as the card's `MealAction::Undo` names it.
    UnskipMeal {
        slot: u8,
        due_ms: i64,
    },
    /// The change the owner made to the table they saw.
    EditPlans {
        plans: PlansView,
        edit: PlanEdit,
    },
    GetCall,
    /// Convert a WAV recording and upload it in place of the stored clip.
    UploadCall {
        wav: Vec<u8>,
    },
    DeleteCall,
    /// Play the stored clip once on the feeder, to hear it.
    PlayCall,
    /// Where an install stands.
    UpdateState,
    /// Stop the install, with bank 1 left invalid.
    CancelUpdate,
    /// Push the feeder's record lines after `after`, as [`SessionUpdate::Log`].
    ///
    /// [`SessionUpdate::Log`]: crate::api::session::SessionUpdate::Log
    TailLog {
        after: Option<u32>,
    },
}

/// A settings change; what is `None` stays.
#[derive(Clone, Copy, Debug, Default)]
pub struct SettingsChanges {
    pub lights: Option<bool>,
    pub sound: Option<bool>,
    pub volume: Option<u8>,
    pub lights_window: Option<Window>,
    pub sound_window: Option<Window>,
    pub desiccant_days: Option<u8>,
    pub meal_reminder: Option<MealReminder>,
}

/// Where the feeder asks the time.
pub enum NtpChoice {
    /// The router that gave it its address.
    Gateway,
    /// Nowhere: it keeps the time phones give it.
    Off,
    /// The server at `address`, an IPv4 address with an optional port. A
    /// word is not an address: `off` and `gateway` are refused here.
    Address { address: String },
}

pub enum ZoneChange {
    /// This phone's zone.
    Phone,
    /// An IANA name.
    Named {
        name: String,
    },
    Clear,
}

pub enum PortionChange {
    Clear,
    /// One portion's weight in milligrams.
    Milligrams {
        mg: u32,
    },
    /// What `portions` counted portions weighed, in grams as typed.
    Weighed {
        grams: String,
        portions: u32,
    },
}

pub enum PlanEdit {
    Add {
        plan: PlanView,
    },
    Change {
        slot: u8,
        minute: Option<u16>,
        portions: Option<u8>,
        days: Option<Vec<Weekday>>,
        call: Option<u8>,
        enabled: Option<bool>,
    },
    Enabled {
        slot: u8,
        enabled: bool,
    },
    Delete {
        slot: u8,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanView {
    pub minute: u16,
    pub portions: u8,
    /// None runs it once, at its next occurrence.
    pub days: Vec<Weekday>,
    pub enabled: bool,
    pub call: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlansView {
    pub generation: u32,
    pub writable: bool,
    /// All ten slots.
    pub plans: Vec<Option<PlanView>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsView {
    pub generation: u32,
    pub lights: bool,
    pub sound: bool,
    pub volume: u8,
    pub lights_window: Window,
    pub sound_window: Window,
    pub desiccant_days: u8,
    /// The feeder's local date of the last replacement, `YYYY-MM-DD`.
    pub desiccant_replaced: Option<String>,
    pub meal_reminder: MealReminder,
    pub writable: bool,
    /// Read-only while the feeder repairs its settings: its two settings
    /// records share no generation, and it takes no change until they do,
    /// at its next restart if not before.
    pub repairing: bool,
}

/// Where the feeder asks for the time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NtpView {
    Off,
    Gateway { port: u16 },
    Address { ip: String, port: u16 },
}

/// A zone rule as it stands now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZoneView {
    /// The abbreviation in force ("MST", "CEST"), or the offset for a rule
    /// that names none ("+03").
    pub abbreviation: String,
    /// Seconds east of UTC.
    pub offset_s: i32,
    /// Whether the rule moves for daylight saving.
    pub seasonal: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeInfo {
    pub generation: u32,
    pub writable: bool,
    /// The zone's POSIX rule.
    pub zone: Option<String>,
    /// The zone as an owner reads it, when the feeder keeps one.
    pub zone_now: Option<ZoneView>,
    /// This phone's zone (an IANA name), when it has one.
    pub phone_zone: Option<String>,
    /// Whether the feeder keeps the same rule as this phone's zone.
    pub same_as_phone: Option<bool>,
    pub ntp: NtpView,
    /// `None` while the clock is invalid: scheduled meals hold.
    pub clock_ms: Option<i64>,
    pub error_ms: Option<u32>,
    pub offset_s: Option<i32>,
    pub syncs: u32,
    pub last: Option<TimeSource>,
    /// When it was last set, by this phone's clock.
    pub last_at_ms: Option<i64>,
    pub ntp_last: Option<NtpResult>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientView {
    pub key: Vec<u8>,
    /// The key's first 4 bytes in hex, as events name a client.
    pub fingerprint: String,
    pub label: String,
    pub role: ClientRole,
    /// Whether this is this phone.
    pub you: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InviteView {
    /// The text a new client redeems: shown once and never logged.
    pub text: String,
    pub seconds: u16,
    pub role: ClientRole,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalibrationInfo {
    pub generation: u32,
    pub writable: bool,
    pub portion_mg: Option<u32>,
    pub bowl: Bowl,
    /// `None` from a feeder with no feeding owner, or before it opens.
    pub phase: Option<Phase>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallInfo {
    pub writable: bool,
    pub clip: CallClip,
    /// How long one play of a stored clip is.
    pub ms: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceView {
    pub summary: TraceSummary,
    pub stored: bool,
    pub entries: Vec<TraceEntry>,
    /// How long each portion took, from each confirmed wheel mark to the last;
    /// empty when the marks do not match the portions counted.
    pub portion_ms: Vec<u32>,
}

/// Where an install stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateInfo {
    pub phase: UpdatePhase,
    /// The bank it answers from.
    pub bank: u8,
    /// The highest confirmed version: an offer at or below it needs a
    /// downgrade statement.
    pub min_version: u32,
    /// The version being installed.
    pub version: u32,
    pub image_len: u32,
    pub written: u32,
    /// Bank 0, the recovery copy, as bank 1 last checked it: blocked means
    /// every update is refused until bank 0 boots once.
    pub bank0: Bank0Check,
}

/// Why the feeder refused a request: the device API's refusals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeederRefusal {
    Forbidden,
    /// The store moved past the base: read it again and decide again.
    /// `current` is the store's generation now.
    Stale {
        current: u32,
    },
    Busy,
    Unavailable,
    Invalid,
    NotWritable,
    Full,
    NotFound,
    Muted,
    Time,
    Fault,
    Update {
        reason: UpdateRefusal,
    },
    Unverified,
    /// A feed outside the portions a feeder takes, as `asked`.
    Portions {
        asked: u8,
    },
    NoClock,
    /// A refusal a newer feeder build added, which this app cannot name.
    Unknown,
}

/// What this phone's Rust half refused before anything was sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotAsked {
    /// The SSID is not 1-32 bytes, or the passphrase not 8-63 printable ASCII.
    Credentials,
    /// That slot holds no plan.
    EmptySlot,
    /// Every slot holds a plan.
    PlansFull,
    /// The change names nothing to change.
    NothingToChange,
    /// A plan's minute, portions or call is outside its limits.
    BadPlan,
    /// The weight is not grams, or counts no portions.
    BadWeight,
    /// The NTP server is not off, gateway or an IPv4 address.
    BadNtp,
    /// The zone is unknown, or changes irregularly.
    BadZone,
    /// The recording is not a WAV file this reads, or is too long once trimmed.
    BadRecording,
    /// A key is not 32 bytes.
    BadKey,
    /// A settings change asked while another was out, which never went: the
    /// one out did not land, so the generation it would go on is not known,
    /// or it was read from another generation than the one out went on.
    Held,
}

/// How a request ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FeederReply {
    /// A write took effect; `generation` is the store's after it, when it has one.
    Done {
        generation: Option<u32>,
    },
    Settings {
        settings: SettingsView,
    },
    Time {
        time: TimeInfo,
    },
    Plans {
        plans: PlansView,
    },
    Clients {
        clients: Vec<ClientView>,
    },
    Invite {
        invite: InviteView,
    },
    Calibration {
        calibration: CalibrationInfo,
    },
    Call {
        call: CallInfo,
    },
    Stats {
        stats: Stats,
    },
    Trace {
        trace: TraceView,
    },
    Update {
        update: UpdateInfo,
    },
    Refused {
        refusal: FeederRefusal,
    },
    /// Refused here, before anything was sent.
    NotAsked {
        reason: NotAsked,
    },
    /// No answer came: a write may have run. It is never sent again.
    Unanswered,
    /// It never went out: nothing happened. `reason` says why.
    NotSent {
        reason: NotSentReason,
    },
    /// The feeder answered something else.
    Unexpected,
    /// A newer feeder build answered in a way this app cannot read. The
    /// request may have run: a write answered so is uncertain, shown so,
    /// and never sent again.
    Unreadable,
}
