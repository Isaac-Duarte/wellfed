//! The IANA zone stock's clock replies are made in. Zones come from the tzdb
//! bundled with this build, never the host's, so the app, the CLI and the
//! bench answer a feeder alike whatever zone data their host carries, and a
//! config written on one reads the same on another.

use std::{fmt, str::FromStr};

use jiff::{Timestamp, tz::TimeZone};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A zone from the bundled tzdb, kept with the name it was given.
#[derive(Clone, Debug)]
pub struct Zone {
    name: &'static str,
    zone: TimeZone,
}

/// A name the bundled tzdb does not hold as written. Names match exactly,
/// case included, so a config keeps the spelling it was written with.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{name:?} is not a time zone in the bundled tzdb")]
pub struct UnknownZone {
    pub name: String,
}

impl Zone {
    pub fn utc() -> Self {
        Self {
            name: "UTC",
            zone: TimeZone::UTC,
        }
    }

    /// The IANA name, as written in a config.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The zone's UTC offset at `at`, in seconds.
    pub fn offset_seconds(&self, at: Timestamp) -> i32 {
        self.zone.to_offset(at).seconds()
    }
}

impl FromStr for Zone {
    type Err = UnknownZone;

    fn from_str(name: &str) -> Result<Self, UnknownZone> {
        let unknown = || UnknownZone { name: name.into() };

        let (canonical, tzif) = jiff_tzdb::get(name).ok_or_else(unknown)?;
        if canonical != name {
            return Err(unknown());
        }
        let zone = TimeZone::tzif(canonical, tzif).map_err(|_| unknown())?;
        Ok(Self {
            name: canonical,
            zone,
        })
    }
}

impl PartialEq for Zone {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for Zone {}

impl fmt::Display for Zone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name)
    }
}

impl Serialize for Zone {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name)
    }
}

impl<'de> Deserialize<'de> for Zone {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        name.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_as_written() {
        for name in [
            "UTC",
            "America/Denver",
            "US/Pacific",
            "Etc/GMT+3",
            "Asia/Calcutta",
        ] {
            let zone: Zone = name.parse().unwrap();
            assert_eq!(zone.name(), name);
            assert_eq!(serde_json::to_value(&zone).unwrap(), name);
        }
    }

    #[test]
    fn unknown_or_respelled_names_are_refused() {
        for name in [
            "Mars/Olympus",
            "america/denver",
            "",
            "+01:00",
            "EST5EDT,M3.2.0,M11.1.0",
        ] {
            assert_eq!(name.parse::<Zone>(), Err(UnknownZone { name: name.into() }));
        }
        assert!(serde_json::from_str::<Zone>(r#""Mars/Olympus""#).is_err());
    }

    #[test]
    fn utc_is_the_bundled_utc() {
        assert_eq!("UTC".parse::<Zone>().unwrap(), Zone::utc());
        assert_eq!(Zone::utc().offset_seconds(Timestamp::now()), 0);
    }
}
