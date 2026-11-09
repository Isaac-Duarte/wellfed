//! `/home`: the home the server serves, which its devices belong to.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::HomeId;

/// How weights are shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Units {
    Grams,
    Ounces,
    /// A value a newer server added, which this client cannot name. The
    /// server never sends it.
    #[serde(other)]
    Unknown,
}

impl Units {
    /// The stored spelling, the same as the JSON one.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Grams => "grams",
            Self::Ounces => "ounces",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        [Self::Grams, Self::Ounces]
            .into_iter()
            .find(|units| units.code() == code)
    }
}

/// How times of day are shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum TimeFormat {
    #[serde(rename = "24h")]
    H24,
    #[serde(rename = "12h")]
    H12,
}

impl TimeFormat {
    /// The stored spelling, the same as the JSON one.
    pub const fn code(self) -> &'static str {
        match self {
            Self::H24 => "24h",
            Self::H12 => "12h",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        [Self::H24, Self::H12]
            .into_iter()
            .find(|format| format.code() == code)
    }
}

/// The home the server serves: one per server for now. Its devices
/// belong to it (each names it as `home`), and its zone is theirs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Home {
    pub id: HomeId,
    /// 1-64 characters.
    pub name: String,
    pub units: Units,
    pub time_format: TimeFormat,
    /// The IANA zone, by its canonical name. Every feeder keeps it as a
    /// POSIX rule, so its clock and plans follow DST with the server down.
    pub zone: String,
}

/// A change to the home; an absent field is left as it is. A new
/// zone goes to every feeder: those offline get it when they reconnect.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct HomePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub units: Option<Units>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_format: Option<TimeFormat>,
    /// An IANA zone; one a POSIX rule cannot describe is refused
    /// `zone_unsupported`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spellings_round_trip() {
        for units in [Units::Grams, Units::Ounces] {
            assert_eq!(Units::from_code(units.code()), Some(units));
            assert_eq!(
                serde_json::to_string(&units).unwrap(),
                format!("\"{}\"", units.code())
            );
        }
        for format in [TimeFormat::H24, TimeFormat::H12] {
            assert_eq!(TimeFormat::from_code(format.code()), Some(format));
            assert_eq!(
                serde_json::to_string(&format).unwrap(),
                format!("\"{}\"", format.code())
            );
        }
    }
}
