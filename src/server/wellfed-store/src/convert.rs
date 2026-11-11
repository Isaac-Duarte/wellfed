//! Conversions between Rust values and the SQLite columns they are stored
//! in. SQLite integers are `i64`, so an unsigned value that does not fit is
//! refused on the way in ([`Error::TooLarge`]) and a stored value out of
//! range is reported on the way out ([`Error::Corrupt`]).

use jiff::Timestamp;
use serde::de::DeserializeOwned;
use wellfed_api::{Hex, Model, Source};

use crate::{Error, Result};

pub(crate) fn millis(at: Timestamp) -> i64 {
    at.as_millisecond()
}

pub(crate) fn timestamp(ms: i64) -> Result<Timestamp> {
    Timestamp::from_millisecond(ms).map_err(|_| Error::Corrupt("time"))
}

/// A device model by the name `wellfed_proto::device` gives it.
pub(crate) fn model(name: &str) -> Result<Model> {
    Model::ALL
        .into_iter()
        .find(|model| model.name() == name)
        .ok_or(Error::Corrupt("device model"))
}

/// A feeder clock time, stored in whole seconds; `None` while that clock
/// was invalid.
pub(crate) fn seconds(at: Option<i64>) -> Result<Option<Timestamp>> {
    at.map(|s| Timestamp::from_second(s).map_err(|_| Error::Corrupt("event time")))
        .transpose()
}

/// Who caused an event, as its two columns: the kind, and in the `client`
/// column the client's fingerprint for a client, or the plan's slot (one
/// byte) for a schedule.
pub(crate) fn source_columns(source: Source) -> (&'static str, Option<Vec<u8>>) {
    match source {
        Source::Panel => ("panel", None),
        Source::Client { client } => ("client", Some(client.0.to_vec())),
        Source::Feeder => ("feeder", None),
        Source::Schedule { plan } => ("schedule", Some(vec![plan])),
        // A newer server's source, which this one never records: its events
        // come from the feeder's wire, where every source has a name.
        Source::Unknown => ("unknown", None),
    }
}

/// The source the two columns name. A kind this server does not know (a
/// newer server's) reads as [`Source::Unknown`]; a known kind whose
/// `client` column does not fit it is damage.
pub(crate) fn source(kind: &str, client: Option<Vec<u8>>) -> Result<Source> {
    const KNOWN: [&str; 4] = ["panel", "feeder", "schedule", "client"];
    if !KNOWN.contains(&kind) {
        return Ok(Source::Unknown);
    }

    match (kind, client) {
        ("panel", None) => Ok(Source::Panel),
        ("feeder", None) => Ok(Source::Feeder),
        ("schedule", Some(slot)) => match slot.as_slice() {
            &[plan] => Ok(Source::Schedule { plan }),
            _ => Err(Error::Corrupt("plan slot")),
        },
        ("client", Some(client)) => {
            let client = <[u8; 4]>::try_from(client.as_slice())
                .map_err(|_| Error::Corrupt("client fingerprint"))?;
            Ok(Source::Client {
                client: Hex(client),
            })
        }
        _ => Err(Error::Corrupt("event source")),
    }
}

pub(crate) fn to_i64(value: u64, what: &'static str) -> Result<i64> {
    i64::try_from(value).map_err(|_| Error::TooLarge(what))
}

pub(crate) fn from_i64(value: i64, what: &'static str) -> Result<u64> {
    u64::try_from(value).map_err(|_| Error::Corrupt(what))
}

/// A cached JSON value and the time it was read, when both are stored. A
/// cache this build doesn't read (a newer build's code in it) reads as
/// absent, as before the feeder was first read.
pub(crate) fn cached<T: DeserializeOwned>(
    json: Option<String>,
    at: Option<i64>,
    what: &'static str,
) -> Result<Option<(T, Timestamp)>> {
    let (Some(json), Some(at)) = (json, at) else {
        return Ok(None);
    };
    let Some(value) = known(&json, what)? else {
        return Ok(None);
    };
    Ok(Some((value, timestamp(at)?)))
}

/// A stored value whose type has an unknown variant: JSON that reads but
/// that this build's type doesn't (a code a newer build wrote, nested
/// anywhere inside it) reads as `unknown`, as the device API reads a code
/// it doesn't know. JSON that doesn't parse at all is damage.
pub(crate) fn lenient<T: DeserializeOwned>(
    json: &str,
    unknown: T,
    what: &'static str,
) -> Result<T> {
    Ok(known(json, what)?.unwrap_or(unknown))
}

/// `json` as `T`, or `None` when it parses but isn't a `T` this build
/// reads.
fn known<T: DeserializeOwned>(json: &str, what: &'static str) -> Result<Option<T>> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|_| Error::Corrupt(what))?;
    Ok(serde_json::from_value(value).ok())
}
