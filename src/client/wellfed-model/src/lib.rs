//! The device API's values as every host works with them: owned, with no
//! lifetimes or wire buffers, so the client's `Connection` can hand them
//! out, the server can store and serve them, and the app can take them
//! through generated bindings ([API design]).
//!
//! `wellfed-proto` is the wire: borrowed, `no_std` and positional, shaped
//! for the feeder. This crate is what a host means by the same values, and
//! [`convert`] is the one place the two meet, so the client, the server
//! and the CLI cannot read a field differently.
//!
//! The JSON forms are the HTTP API's (`wellfed-api` serves these types
//! as they are, and its OpenAPI schemas come from their derives): fields
//! `snake_case`, times RFC 3339, keys and fingerprints lowercase hex.
//!
//! [API design]: ../../../../docs/server/design.md#crates
#![forbid(unsafe_code)]

mod alerts;
mod call;
pub mod convert;
mod event;
mod plan;
mod request;
mod session;
pub mod setup;
mod stats;
mod status;
mod trace;
mod update;

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use utoipa::ToSchema;

pub use alerts::{Alert, AlertKey, AlertKind, AlertLevel, AlertsView, OpenAlert};
pub use call::{CALL_BYTES_MAX, CALL_CHUNK_MAX, CALL_PLAYS_MAX, CALL_RATE_HZ, CallClip, CallView};
pub use event::{
    ClientRole, CoreEvent, EnrollPath, Event, EventBody, EventKind, FeedOutcome, FeedReason,
    FeederEvent, FoodLevel, Gap, LogLines, MetricKind, MissedReason, Pushed, Sample, SampleValue,
    Source, UnknownEvent,
};
pub use plan::{
    NextMeal, PLANS, PORTIONS_MAX, PORTIONS_MIN, Plan, Plans, SkippedMeals, TimeOfDay, Weekday,
    Weekdays, WeekdaysError,
};
pub use request::{
    Answer, Bowl, Calibration, CalibrationView, Client, Invite, NtpResult, NtpServer, Op, Phase,
    Refusal, TimeChanges, TimeSource, TimeSync, TimeView, Tune, Wifi,
};
pub use session::{
    DeviceKind, DeviceName, Hello, Joined, LEGACY_SCHEMA_FROM, MaybeKnown, Model, OpenRefusal,
    Skew, schema,
};
pub use stats::{
    Counters, Counts, Erases, FeedStats, Flash, Food, Memory, Portions, PowerStats, ResetCause,
    Resets, Stats, StatsSection, TaskStack,
};
pub use status::{
    Battery, Blocked, Desiccant, DeviceStatus, FeedFault, FeederSettings, FeederSettingsPatch,
    FeederStatus, MealReminder, MealReminderError, Power, Readiness, Settings, Status, Supply,
    WifiSource, Window, WindowError,
};
pub use trace::{CurrentUnit, Malformed, Trace, TraceEntry, TraceMotor, TracePage, TraceSummary};
pub use update::{
    Bank0Blocked, Bank0Check, Bank1Page, UpdateCandidate, UpdatePhase, UpdateRefusal, UpdateStop,
    UpdateView,
};
/// The clients a feeder lists at most, owners, members and the server
/// together.
pub use wellfed_proto::api::CLIENTS_MAX;
/// What a feeder's settings report as their generation while it has none.
pub use wellfed_proto::api::NO_SETTINGS_GENERATION;
/// The desiccant reminder's period limits and default, in days.
pub use wellfed_proto::api::{
    DESICCANT_DAYS_DEFAULT, DESICCANT_DAYS_MAX, DESICCANT_DAYS_MIN, MEAL_REMINDER_MINUTES_MAX,
    MEAL_REMINDER_MINUTES_MIN,
};
/// The portion weights a calibration may store, in milligrams.
pub use wellfed_proto::api::{PORTION_MG_MAX, PORTION_MG_MIN};

/// A feeder store's generation: a write names the one it read, and the
/// feeder refuses it once the store has moved on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(transparent)]
#[schema(value_type = u32)]
pub struct Generation(pub u32);

impl fmt::Display for Generation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Generation {
    /// A feeder's settings generation as it reports it: `None` for
    /// [`wellfed_proto::api::NO_SETTINGS_GENERATION`], which it reports,
    /// read-only, while its two settings records share none.
    pub const fn settings(raw: u32) -> Option<Self> {
        if raw == wellfed_proto::api::NO_SETTINGS_GENERATION {
            None
        } else {
            Some(Self(raw))
        }
    }
}

/// A settings generation that may be none, in JSON as the feeder sends it:
/// a number always, [`wellfed_proto::api::NO_SETTINGS_GENERATION`] for
/// none, so `/api/v1` keeps the field's type for older readers.
pub(crate) mod settings_generation {
    use serde::{Deserialize, Deserializer, Serializer};

    use super::Generation;

    pub(crate) fn serialize<S: Serializer>(
        generation: &Option<Generation>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let raw = generation.map_or(wellfed_proto::api::NO_SETTINGS_GENERATION, |g| g.0);
        serializer.serialize_u32(raw)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Generation>, D::Error> {
        u32::deserialize(deserializer).map(Generation::settings)
    }
}

/// Bytes that travel as lowercase hex: a feeder's 32-byte identity key or a
/// client's 4-byte fingerprint. Fields of this type carry
/// `#[schema(value_type = String)]`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hex<const N: usize>(pub [u8; N]);

impl<const N: usize> fmt::Display for Hex<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        wellfed_session::hex::Lower(&self.0).fmt(f)
    }
}

impl<const N: usize> fmt::Debug for Hex<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl<const N: usize> Serialize for Hex<N> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de, const N: usize> Deserialize<'de> for Hex<N> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        wellfed_session::hex::decode_array(text.as_bytes())
            .map(Self)
            .ok_or_else(|| serde::de::Error::custom(format!("not {N} bytes of lowercase hex")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_lowercase_and_exact_length() {
        let json = serde_json::to_string(&Hex([0xab, 0x01])).unwrap();
        assert_eq!(json, "\"ab01\"");
        assert_eq!(
            serde_json::from_str::<Hex<2>>(&json).unwrap(),
            Hex([0xab, 0x01])
        );
        assert!(serde_json::from_str::<Hex<2>>("\"AB01\"").is_err());
        assert!(serde_json::from_str::<Hex<2>>("\"ab\"").is_err());
    }

    /// The server stores these codes, and its JSON shows the same values,
    /// so the two spellings never drift apart.
    #[test]
    fn stored_codes_are_the_json_spellings() {
        fn check<T: Serialize + Copy>(all: &[T], code: fn(T) -> &'static str) {
            for &value in all {
                let json = serde_json::to_string(&value).unwrap();
                assert_eq!(json, format!("\"{}\"", code(value)));
            }
        }

        check(&FeedOutcome::ALL, FeedOutcome::code);
        check(&FeedReason::ALL, FeedReason::code);
        check(&Blocked::ALL, Blocked::code);
        check(&EventKind::ALL, EventKind::code);
        for kind in EventKind::ALL {
            assert_eq!(EventKind::from_code(kind.code()), Some(kind));
        }
        for outcome in FeedOutcome::ALL {
            assert_eq!(FeedOutcome::from_code(outcome.code()), Some(outcome));
        }
        for reason in FeedReason::ALL {
            assert_eq!(FeedReason::from_code(reason.code()), Some(reason));
        }
        for blocked in Blocked::ALL {
            assert_eq!(Blocked::from_code(blocked.code()), Some(blocked));
        }
    }
}
