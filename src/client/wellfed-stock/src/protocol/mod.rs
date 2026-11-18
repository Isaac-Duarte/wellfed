//! The PETLIBRO cloud protocol stock firmware speaks, as every model shares
//! it: the topic scheme, the channels, the message envelope, the clock
//! reply and the command envelope. Each model's own messages (what the
//! server answers and what it sends unasked) are a module of their own:
//! [`plaf103`] is the feeder's, and the only one so far.
//!
//! A device posts JSON on `dl/<model>/<device_id>/device/<channel>/post` and
//! the server answers on the matching `/sub` topic.
//!
//! Replies are typed structs, but go out through a `Value` so their keys are
//! sorted exactly as the captured server's were (`tests/wire.rs` pins them).

use std::fmt;

use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use uuid::Uuid;
use wellfed_formats::require;

use wellfed_model::Model;

use crate::config::Config;
use crate::zone::Zone;

pub mod plaf103;

/// A model's stock message set: what its stock firmware speaks over this
/// core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageSet {
    /// [`plaf103`].
    Plaf103,
}

impl MessageSet {
    /// The message set of `model`'s stock firmware, `None` while it has
    /// none: the PLWF106's waits on its reverse engineering.
    pub const fn of(model: Model) -> Option<Self> {
        match model {
            Model::Plaf103 => Some(Self::Plaf103),
            Model::Plwf106 => None,
        }
    }
}

/// The topic under `device`'s namespace in model `pid`'s tree (`PLAF103`):
/// what stock's own topics and the broker's checks are built from.
pub fn device_topic(pid: &str, device: &str, suffix: &str) -> String {
    format!("dl/{pid}/{device}/device/{suffix}")
}

/// The topic channels the stock firmware posts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    System,
    Ntp,
    Event,
    Service,
    Heart,
    Ota,
    Config,
}

impl Channel {
    const ALL: [Self; 7] = [
        Self::System,
        Self::Ntp,
        Self::Event,
        Self::Service,
        Self::Heart,
        Self::Ota,
        Self::Config,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Ntp => "ntp",
            Self::Event => "event",
            Self::Service => "service",
            Self::Heart => "heart",
            Self::Ota => "ota",
            Self::Config => "config",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|channel| channel.name() == name)
    }

    /// Where the server answers or commands the device on this channel.
    pub fn sub_topic(self, config: &Config) -> String {
        config.topic(&format!("{self}/sub"))
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A field read as `T`, or `None` when it is of another type: one odd field
/// must not drop a whole stock message (or capture record).
pub(crate) fn lenient<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned,
{
    let value = Value::deserialize(deserializer)?;
    Ok(T::deserialize(value).ok())
}

/// The fields of a device message the server acts on, each read leniently.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    #[serde(default, deserialize_with = "lenient")]
    pub software_version: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub power_type: Option<u64>,
    #[serde(default, deserialize_with = "lenient")]
    pub code: Option<i64>,
    #[serde(default, deserialize_with = "lenient")]
    pub err_code: Option<u64>,
    #[serde(default, deserialize_with = "lenient")]
    pub phase: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub state: Option<u8>,
    #[serde(default, deserialize_with = "lenient")]
    pub progress: Option<u8>,
}

#[derive(Deserialize)]
struct Envelope {
    cmd: String,
    #[serde(rename = "msgId")]
    msg_id: Option<String>,
    ts: Option<i64>,
    #[serde(flatten)]
    fields: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(from = "Envelope")]
pub struct Message {
    pub cmd: String,
    pub msg_id: Option<String>,
    pub ts: Option<i64>,
    pub report: Report,
    /// Every other field as sent, for the telemetry allowlist and echoes.
    pub fields: Map<String, Value>,
}

impl From<Envelope> for Message {
    fn from(envelope: Envelope) -> Self {
        // Every field is lenient, so only a non-map could fail, and this is one.
        let report = Report::deserialize(&envelope.fields).unwrap_or_default();

        Self {
            cmd: envelope.cmd,
            msg_id: envelope.msg_id,
            ts: envelope.ts,
            report,
            fields: envelope.fields,
        }
    }
}

impl Message {
    /// The message ID as a request UUID, for correlating with the ledger.
    pub fn request_id(&self) -> Option<Uuid> {
        Uuid::parse_str(self.msg_id.as_deref()?).ok()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Reply {
    pub topic: String,
    pub payload: Value,
}

impl Reply {
    pub(crate) fn new(topic: String, payload: &impl Serialize) -> Self {
        let payload = serde_json::to_value(payload)
            .expect("reply structs have string keys and infallible fields");
        Self { topic, payload }
    }
}

/// A device message the server ignores. Device text never reaches a
/// message: a topic or payload is the device's to make up.
#[derive(Debug, thiserror::Error)]
pub enum MessageError {
    #[error("device payload of {bytes} bytes is too large")]
    TooLarge { bytes: usize },
    #[error("unexpected device topic")]
    Topic,
    #[error("expected a device post topic")]
    NotPost,
    #[error("unknown topic channel")]
    Channel,
    #[error("invalid device JSON")]
    Json(#[source] serde_json::Error),
    #[error("invalid command name")]
    Command,
}

pub fn parse(
    config: &Config,
    topic: &str,
    payload: &[u8],
) -> Result<(Channel, Message), MessageError> {
    require(payload.len() <= 65_536, || MessageError::TooLarge {
        bytes: payload.len(),
    })?;
    let prefix = config.topic("");
    let suffix = topic.strip_prefix(&prefix).ok_or(MessageError::Topic)?;
    let channel = suffix.strip_suffix("/post").ok_or(MessageError::NotPost)?;
    let channel = Channel::from_name(channel).ok_or(MessageError::Channel)?;

    let message: Message = serde_json::from_slice(payload).map_err(MessageError::Json)?;
    require(
        !message.cmd.is_empty()
            && message.cmd.len() <= 64
            && message
                .cmd
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'),
        || MessageError::Command,
    )?;

    Ok((channel, message))
}

pub(crate) fn link_id() -> String {
    Uuid::new_v4().simple().to_string()
}

/// The stock `NTP` reply: the zone's current offset and its next two changes.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ntp {
    pub calibration_tag: bool,
    pub cmd: &'static str,
    pub code: u8,
    pub link_id: String,
    #[serde(rename = "nextDSTOffsetSeconds")]
    pub next_dst_offset_seconds: i32,
    /// Milliseconds; 0 when the zone has no change ahead.
    #[serde(rename = "nextDSTTransitionTs")]
    pub next_dst_transition_ts: i64,
    #[serde(rename = "secondNextDSTOffsetSeconds")]
    pub second_next_dst_offset_seconds: i32,
    #[serde(rename = "secondNextDSTTransitionTs")]
    pub second_next_dst_transition_ts: i64,
    /// Hours.
    pub timezone: f64,
    pub timezone_offset_seconds: i32,
    pub ts: i64,
}

/// How far ahead the stock reply looks for the zone's next two changes.
const NTP_HORIZON: SignedDuration = SignedDuration::from_hours(800 * 24);

pub fn ntp_response(zone: &Zone, now: Timestamp, calibrate: bool) -> Ntp {
    let current = zone.offset_seconds(now);
    let horizon = now + NTP_HORIZON;

    let first = next_offset_change(zone, now, horizon);
    let second = first.and_then(|(at, _)| next_offset_change(zone, at, horizon));
    let wire = |change: Option<(Timestamp, i32)>| match change {
        Some((at, offset)) => (offset, at.as_millisecond()),
        None => (current, 0),
    };
    let (next_offset, next_ts) = wire(first);
    let (second_offset, second_ts) = wire(second);

    Ntp {
        calibration_tag: calibrate,
        cmd: "NTP",
        code: 0,
        link_id: link_id(),
        next_dst_offset_seconds: next_offset,
        next_dst_transition_ts: next_ts,
        second_next_dst_offset_seconds: second_offset,
        second_next_dst_transition_ts: second_ts,
        timezone: f64::from(current) / 3600.0,
        timezone_offset_seconds: current,
        ts: now.as_millisecond(),
    }
}

/// The first whole second after `after`, up to `until`, with a different UTC
/// offset, and that offset. UTC days are scanned, then the exact second is
/// found by bisection, so no ambiguous local time is involved.
pub(crate) fn next_offset_change(
    zone: &Zone,
    after: Timestamp,
    until: Timestamp,
) -> Option<(Timestamp, i32)> {
    let offset_at = |seconds: i64| {
        Timestamp::from_second(seconds)
            .ok()
            .map(|at| zone.offset_seconds(at))
    };
    let current = offset_at(after.as_second())?;

    let mut low = after.as_second();
    let mut high = None;
    while low < until.as_second() {
        let end = (low + 86_400).min(until.as_second());
        if offset_at(end)? != current {
            high = Some(end);
            break;
        }
        low = end;
    }

    let mut high = high?;
    while high - low > 1 {
        let middle = low + (high - low) / 2;
        if offset_at(middle)? == current {
            low = middle;
        } else {
            high = middle;
        }
    }

    let at = Timestamp::from_second(high).ok()?;
    Some((at, zone.offset_seconds(at)))
}

/// A command carrying only the stock envelope fields.
#[derive(Serialize)]
pub(crate) struct Plain {}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Command<'a, T> {
    cmd: &'a str,
    msg_id: String,
    ts: i64,
    #[serde(flatten)]
    body: &'a T,
}

/// The server command `cmd` with request `id` on `channel`; `body` adds its
/// fields beside the envelope's. Each model's message set names its own
/// commands (`plaf103::Sent`).
pub fn command(
    config: &Config,
    channel: Channel,
    cmd: &str,
    id: Uuid,
    now: Timestamp,
    body: &impl Serialize,
) -> Reply {
    let command = Command {
        cmd,
        msg_id: id.to_string(),
        ts: now.as_millisecond(),
        body,
    };
    Reply::new(channel.sub_topic(config), &command)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-10-17T21:17:41Z, when the Denver reply was captured.
    fn captured_now() -> Timestamp {
        Timestamp::from_second(1_792_271_861).unwrap()
    }

    #[test]
    fn denver_time_response_matches_captured_dst_transitions() {
        let denver = "America/Denver".parse().unwrap();
        let response = ntp_response(&denver, captured_now(), true);

        assert_eq!(response.timezone_offset_seconds, -21600);
        assert_eq!(response.next_dst_offset_seconds, -25200);
        assert_eq!(response.next_dst_transition_ts, 1793520000000_i64);
        assert_eq!(response.second_next_dst_offset_seconds, -21600);
        assert_eq!(response.second_next_dst_transition_ts, 1805014800000_i64);
        assert_eq!(response.ts, 1_792_271_861_000);
    }

    /// Southern, half-hour, half-hour-shift and DST-ending zones, as the
    /// reply gave them when it was built on chrono-tz: a change of zone
    /// library must not move a feeder's clock.
    #[test]
    fn other_zones_keep_their_offsets_and_changes() {
        let cases: [(&str, f64, [i64; 5]); 5] = [
            (
                "Australia/Sydney",
                11.0,
                [39600, 36000, 1806768000000, 39600, 1822492800000],
            ),
            ("Asia/Kolkata", 5.5, [19800, 19800, 0, 19800, 0]),
            (
                "Europe/Lisbon",
                1.0,
                [3600, 0, 1792890000000, 3600, 1806195600000],
            ),
            (
                "America/Santiago",
                -3.0,
                [-10800, -14400, 1806807600000, -10800, 1820116800000],
            ),
            (
                "Australia/Lord_Howe",
                11.0,
                [39600, 37800, 1806764400000, 39600, 1822491000000],
            ),
        ];

        for (name, hours, [current, next, next_ts, second, second_ts]) in cases {
            let zone = name.parse().unwrap();
            let response = ntp_response(&zone, captured_now(), true);

            assert_eq!(response.timezone, hours, "{name}");
            assert_eq!(
                i64::from(response.timezone_offset_seconds),
                current,
                "{name}"
            );
            assert_eq!(i64::from(response.next_dst_offset_seconds), next, "{name}");
            assert_eq!(response.next_dst_transition_ts, next_ts, "{name}");
            assert_eq!(
                i64::from(response.second_next_dst_offset_seconds),
                second,
                "{name}"
            );
            assert_eq!(response.second_next_dst_transition_ts, second_ts, "{name}");
        }
    }

    #[test]
    fn zones_without_dst_do_not_invent_transition_dates() {
        let response = ntp_response(&Zone::utc(), Timestamp::now(), false);

        assert_eq!(response.next_dst_transition_ts, 0);
        assert!(!response.calibration_tag);
    }

    #[test]
    fn odd_fields_are_skipped_without_dropping_the_message() {
        let message: Message = serde_json::from_value(serde_json::json!({
            "cmd": "OTA_PROGRESS",
            "softwareVersion": 7,
            "progress": "half",
            "powerType": 1,
            "state": 300,
        }))
        .unwrap();

        assert_eq!(message.report.software_version, None);
        assert_eq!(message.report.progress, None);
        assert_eq!(message.report.state, None);
        assert_eq!(message.report.power_type, Some(1));
        assert_eq!(message.fields["softwareVersion"], 7);
    }
}
