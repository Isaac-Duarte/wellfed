//! What the owner's server answers, as Dart reads it. `wellfed-api`'s
//! bodies carry times, dates and ids Dart cannot take as they are, so each
//! one with those has a view here: ids are their UUID strings, times
//! milliseconds since the epoch, and dates `YYYY-MM-DD`. A body made only of
//! numbers and plain enums is mirrored, so flutter_rust_bridge generates its
//! Dart type from the API itself. [`crate::server`] converts.
//!
//! Feeder history reads as the History tab's [`HistoryEntry`], and a
//! feeder's status as the Home [`Card`], so a screen draws the server's
//! answer and the feeder's own with one widget.

use flutter_rust_bridge::frb;
pub use wellfed_api::{
    CommandKind, CommandState, FeedRate, InstallState, PortionWeight, Species, WeighedIntake,
};

pub use crate::api::model::{ClientRole, FeedOutcome, FeedReason, MissedReason};
use crate::api::session::{Card, HistoryEntry};

/// What the app keeps of a server in its own file: where it is, and which
/// token is this phone's. The token's secret is not here; it lives in
/// secure storage beside the client secret.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerLink {
    /// The base URL, as the owner typed it or discovery found it.
    pub url: String,
    /// The account's name, for the account screen while offline.
    pub user: String,
    /// The phone token's id, which logging out revokes.
    pub token_id: String,
}

/// The logged-in account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerAccount {
    pub id: String,
    pub name: String,
    pub role: ServerRole,
    /// When the account stops; `None` for good.
    pub expires_ms: Option<i64>,
}

/// What an account may do in the home.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerRole {
    Owner,
    Member,
    Viewer,
}

/// The server's link to one feeder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerConnection {
    Offline,
    Connecting,
    Online,
    /// The feeder refused the server's key: removed or reset.
    Refused,
}

/// One adopted feeder, as `GET /feeders` lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerFeeder {
    pub id: String,
    pub name: String,
    /// The feeder's identity key, lowercase hex: what the phone's store and
    /// NSD's `id` match.
    pub identity: String,
    /// The server's own role on the feeder.
    pub role: ClientRole,
    pub address: Option<String>,
    pub connection: ServerConnection,
    /// The newest event sequence the server holds.
    pub synced: Option<u64>,
    pub epoch: Option<u32>,
    /// When it was removed from the server; its history stays readable.
    pub archived_ms: Option<i64>,
}

/// One feeder with its last status and next meal.
pub struct ServerFeederDetail {
    pub feeder: ServerFeeder,
    pub status: Option<ServerSnapshot>,
    pub next_meal: Option<ServerNextMeal>,
}

/// The feeder's status as the server last heard it.
pub struct ServerSnapshot {
    pub at_ms: i64,
    /// Whether the server's session is open now; a stale snapshot is the
    /// last one before it dropped.
    pub live: bool,
    /// The status as the Home card reads it. The server's answer carries no
    /// plans, settings or time read, so those parts are empty.
    pub card: Card,
}

/// The plans' next meal, by the feeder's clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServerNextMeal {
    pub at_ms: i64,
    pub slot: u8,
    pub portions: u8,
}

/// A page of a feeder's history, oldest first, events and gaps together.
pub struct ServerEventPage {
    /// The history these belong to; `None` before the server has read any.
    pub epoch: Option<u32>,
    pub entries: Vec<ServerEntry>,
    /// The `after` of the next page; `None` once this is the newest.
    pub next: Option<u64>,
}

/// One history entry, with when the server received it.
pub struct ServerEntry {
    pub epoch: u32,
    pub received_ms: i64,
    pub entry: HistoryEntry,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerFood {
    pub id: String,
    pub brand: String,
    pub name: String,
    pub kcal_per_kg: u32,
    pub bag_grams: u32,
}

/// A food a feeder held, from `from_ms` until `to_ms` (`None`: still).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerFoodPeriod {
    pub food: ServerFood,
    pub from_ms: i64,
    pub to_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerRefill {
    pub id: String,
    pub feeder: String,
    pub at_ms: i64,
    pub grams: u32,
    pub full_bag: bool,
    pub food: Option<String>,
    pub user: String,
}

/// A feeder's food view: what it holds, how much is left and how fast it
/// goes, by the feeder's portion calibration alone. Uncalibrated, it counts
/// portions only.
pub struct ServerFeederFood {
    pub current: Option<ServerFoodPeriod>,
    pub last_refill: Option<ServerRefill>,
    pub weight: PortionWeight,
    pub since_refill: Option<ServerSinceRefill>,
    pub rate: Option<FeedRate>,
    /// When the feeder saw food return with no refill recorded since.
    pub refilled_unrecorded_ms: Option<i64>,
    pub anomalies: Vec<ServerAnomaly>,
}

pub struct ServerSinceRefill {
    pub portions: u32,
    pub uncertain_portions: u32,
    /// Only with a calibration and a recorded refill's weight.
    pub left: Option<ServerFoodLeft>,
}

pub struct ServerFoodLeft {
    pub grams_used: u32,
    pub grams_left: u32,
    /// As if every uncertain portion fell.
    pub grams_left_least: u32,
    pub empty_at_ms: Option<i64>,
}

/// A feed that went other than planned.
pub enum ServerAnomaly {
    Missed {
        feeder: String,
        plan: u8,
        due_ms: i64,
        reason: MissedReason,
    },
    Uncertain {
        feeder: String,
        feed: u32,
        at_ms: i64,
        requested: u8,
        completed: u8,
    },
    Short {
        feeder: String,
        feed: u32,
        at_ms: i64,
        requested: u8,
        completed: u8,
        outcome: FeedOutcome,
        reason: FeedReason,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerPet {
    pub id: String,
    pub name: String,
    pub species: Species,
    /// `YYYY-MM-DD`.
    pub birth_date: Option<String>,
    pub daily_kcal: Option<u32>,
    /// The feeders it eats from, by id.
    pub feeders: Vec<String>,
}

/// A pet's intake, day by day in the home's zone.
pub struct ServerPetIntake {
    pub pet: String,
    pub zone: String,
    pub daily_kcal: Option<u32>,
    pub feeders: Vec<ServerIntakeFeeder>,
    /// Oldest first.
    pub days: Vec<ServerIntakeDay>,
    pub anomalies: Vec<ServerAnomaly>,
}

pub struct ServerIntakeFeeder {
    pub feeder: String,
    pub weight: PortionWeight,
    /// Another pet eats from it too, so its portions are an estimate.
    pub shared: bool,
}

pub struct ServerIntakeDay {
    /// `YYYY-MM-DD`.
    pub date: String,
    pub weighed: Option<WeighedIntake>,
    pub target_percent: Option<u32>,
    pub estimate: bool,
    /// Portions from feeders with no calibration, which weigh nothing.
    pub unweighed: Vec<ServerFeederPortions>,
}

pub struct ServerFeederPortions {
    pub feeder: String,
    pub portions: u32,
    pub uncertain_portions: u32,
}

#[frb(mirror(Species))]
pub enum _Species {
    Cat,
    Dog,
    Other,
}

#[frb(mirror(PortionWeight))]
pub enum _PortionWeight {
    Calibration { portion_mg: u32 },
    Uncalibrated,
}

#[frb(mirror(FeedRate))]
pub struct _FeedRate {
    pub portions: u32,
    pub hours: u32,
    pub grams_per_day: Option<u32>,
}

#[frb(mirror(WeighedIntake))]
pub struct _WeighedIntake {
    pub grams: u32,
    pub kcal: u32,
    pub uncertain_grams: u32,
    pub unknown_food_grams: u32,
}

#[frb(mirror(CommandKind))]
pub enum _CommandKind {
    Settings,
    Lock,
    Feed,
    Cancel,
    Invite,
    Revoke,
    Wifi,
    Reboot,
    Play,
    Remove,
    Desiccant,
    Plans,
    Calibration,
    CallUpload,
    CallDelete,
    CallPlay,
}

#[frb(mirror(CommandState))]
pub enum _CommandState {
    Sent,
    Answered,
    Unknown,
    Resolved,
}

#[frb(mirror(InstallState))]
pub enum _InstallState {
    Running,
    Activated,
    AlreadyRunning,
    Failed,
}
