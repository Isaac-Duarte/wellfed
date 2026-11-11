//! The owner's server HTTP API ([API design]): every request and response
//! body under `/api/v1`, its error codes and its OpenAPI schemas.
//!
//! The server serializes these, and the app (through generated bindings)
//! and tests deserialize them, so the crate builds for every target and
//! carries no server dependency. Clients outside Rust take their types from
//! the `openapi.json` the server generates from them.
//!
//! Fields are `snake_case`, IDs UUIDv7, and times RFC 3339. Feeder keys and
//! fingerprints travel as lowercase hex.
//!
//! [API design]: ../../../../docs/server/README.md#the-http-api
#![forbid(unsafe_code)]

mod alerts;
mod audit;
mod auth;
mod backups;
mod clients;
mod commands;
mod diagnostics;
mod error;
mod events;
mod export;
mod feeders;
mod foods;
mod health;
mod home;
mod installs;
mod pets;
mod plain_http;
mod releases;
mod samples;
mod stock;
mod stream;
mod tokens;
mod users;
mod views;

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

pub use alerts::{AlertsRead, FeederAlerts};
pub use audit::{AuditAction, AuditEntry, AuditPage, LoginFailure};
pub use auth::{Credentials, PasswordChange, Role, Setup, SetupState, User};
pub use backups::{Backup, BackupPage};
pub use clients::{
    ClientOwner, FeederClient, KeyRemoval, NewPhoneInvite, PhoneClaim, PhoneInvite, PhoneKey,
    PlayRequest, Removal, RevokeAttempt, RevokeReason, Revoking, TuneName, WifiChange,
    WifiCredentials,
};
pub use commands::{
    Command, CommandKind, CommandOutcome, CommandPage, CommandQuery, CommandState,
    CommandStateFilter, FeedRequest,
};
pub use diagnostics::{FeedTrace, FeederStats, LogEvent, StatsRead, TraceQuery, TraceRecord};
pub use error::{ErrorCode, FeederRefusal, Problem};
pub use events::{EventPage, EventQuery, Gap, HistoryEvent};
pub use export::{ExportFormat, ExportQuery, Imported, ImportedTable};
pub use feeders::{
    AdoptFeeder, Connection, Feeder, FeederDetail, LockRequest, MealRequest, PlansChange,
    StatusSnapshot,
};
pub use foods::{
    FeederFood, Food, FoodChoice, FoodHistoryPage, FoodPatch, FoodPeriod, NewFood, NewRefill,
    Refill, RefillPage,
};
pub use health::{
    FeederLink, Health, Job, JobOutcome, JobRun, ServerSkew, ServerStatus, StoreState,
};
pub use home::{Home, HomePatch, TimeFormat, Units};
pub use installs::{Install, InstallFailure, InstallRequest, InstallState, InstallTarget};
// The feeder's own values are the model's, served as they are; the HTTP
// API adds only what the server knows (when it received them, its own ids).
pub use pets::{NewPet, NewWeight, Pet, PetLink, PetPatch, Species, Weight, WeightPage};
pub use plain_http::{PlainHttp, at_home};
pub use releases::{
    AssetName, RefusedAsset, Release, ReleaseCheck, ReleasePage, ReleaseQuery, ReleaseRefusal,
    ReleaseSigner, ReleaseSource,
};
pub use samples::{
    HourSample, RawSample, Resolution, SAMPLE_NUMBER_VERSION, SamplePage, SampleQuery,
    sample_number,
};
pub use stock::{
    MANIFEST_LEN, STATEMENT_LEN, StockBackup, StockBackupState, StockFeeder, StockRestore,
    StockSource, StockState,
};
pub use stream::{FeederSample, StreamEvent, StreamQuery};
pub use tokens::{BEARER, CreatedToken, NewToken, Token, TokenId};
pub use users::{NewUser, UserPatch};
pub use views::{
    FOOD_VIEW_DAYS, FeedAnomaly, FeedRate, FeederPortions, FoodLeft, INTAKE_DAYS, INTAKE_DAYS_MAX,
    IntakeDay, IntakeFeeder, IntakeQuery, PetIntake, PortionWeight, SinceRefill, WeighedIntake,
};
pub use wellfed_model::{
    Alert, AlertKey, AlertKind, AlertLevel, AlertsView, Battery, Blocked, Bowl, CALL_BYTES_MAX,
    Calibration, CalibrationView, CallClip, CallView, ClientRole, CoreEvent, CurrentUnit,
    Desiccant, DeviceKind, DeviceStatus, EnrollPath, EventBody, EventKind, FeedOutcome, FeedReason,
    FeederEvent, FeederSettings, FeederSettingsPatch, FeederStatus, Food as FoodStats, FoodLevel,
    Generation, Hex, MaybeKnown, MealReminder, MetricKind, MissedReason, Model, NextMeal,
    OpenAlert, PLANS, PORTION_MG_MAX, PORTION_MG_MIN, Phase, Plan, Plans, Power, Readiness,
    SampleValue, Settings, Source, Stats, StatsSection, Status, Supply, Trace, TraceEntry,
    TraceSummary, Weekday, Weekdays, WifiSource, Window,
};

/// The prefix every resource lives under.
pub const PREFIX: &str = "/api/v1";

/// The ordered revision of the API under [`PREFIX`], which `GET /health`
/// answers. From the beta the API only grows: no field is removed, retyped
/// or made required, a body gains only optional fields, and an enum gains
/// variants only at its end, and only one with a catch-all (an `unknown`
/// value its older readers decode a newer one as). Each growth raises this
/// by one. `wellfed-server`'s `api_growth` test holds the published
/// document to that ([API evolution]).
///
/// [API evolution]: ../../../../docs/server/README.md#api-evolution
pub const REVISION: u32 = 4;

/// The cookie a login sets: an opaque session token, `HttpOnly` and
/// `SameSite=Strict`. A non-browser client sends it back as it came.
pub const SESSION_COOKIE: &str = "wellfed_session";

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, ToSchema,
)]
#[serde(transparent)]
#[schema(value_type = String, format = Uuid)]
pub struct FeederId(pub Uuid);

impl fmt::Display for FeederId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Ids the server gives: UUIDv7s that serialize as their strings.
macro_rules! ids {
    ($($(#[$doc:meta])* $name:ident),* $(,)?) => {$(
        $(#[$doc])*
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
            ToSchema,
        )]
        #[serde(transparent)]
        #[schema(value_type = String, format = Uuid)]
        pub struct $name(pub Uuid);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    )*};
}

ids!(
    /// The home the server serves, which its devices belong to.
    HomeId,
    UserId,
    PetId,
    /// One entry of a pet's weight log.
    WeightId,
    FoodId,
    RefillId,
    InstallId,
);

/// A page of a list that grows: the previous page's `next`, and a size.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PageQuery {
    /// The previous page's `next`, as it came.
    pub after: Option<String>,
    /// At most this many (1-1000, default 100).
    pub limit: Option<u32>,
}

/// A field that is present is `Some`, even when it is `null`: absent
/// leaves a value alone, `null` clears it.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}
