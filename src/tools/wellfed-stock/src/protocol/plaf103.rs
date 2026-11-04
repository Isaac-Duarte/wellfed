//! Stock PLAF103 1.1.15's message set over the shared protocol: what the
//! server answers and what it sends unasked. Only captured commands get a
//! reply (clock, binding, the imported feeding-plan snapshot and event
//! acknowledgements); schedules are served from the snapshot, never edited.

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Map, Value};
use uuid::Uuid;

use super::{Channel, Message, Plain, Reply, command, link_id, ntp_response};
use crate::clock::{self, StockPlans};
use crate::config::Config;

/// The feeder's product id: its topic tree's name and an install's
/// `targetPid`.
pub const PID: &str = "PLAF103";

/// The server's acknowledgement of a device event.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Ack<'a> {
    cmd: &'a str,
    code: u8,
    /// Echoed exactly as the device sent it.
    #[serde(skip_serializing_if = "Option::is_none")]
    exec_step: Option<&'a Value>,
    link_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    msg_id: Option<&'a str>,
    /// The imported snapshot, replayed verbatim.
    #[serde(skip_serializing_if = "Option::is_none")]
    plans: Option<&'a [Value]>,
    ts: i64,
}

/// The `BINDING` reply of a server with no captured one: bound, under an
/// association only this server knows, and with no camera to authorize.
fn installer_binding() -> Map<String, Value> {
    let mut reply = Map::new();
    reply.insert("cmd".into(), "BINDING".into());
    reply.insert("code".into(), 0.into());
    reply.insert("bindId".into(), 1.into());
    reply.insert("cameraAuthInfo".into(), "".into());
    reply
}

/// The reply to `msg`, if it gets one. `clock` is the owner's policy, which
/// with an `NTP` request's own `ts` decides whether the reply sets stock's
/// clock ([`crate::clock`]).
pub fn reply(
    config: &Config,
    channel: Channel,
    msg: &Message,
    now: DateTime<Utc>,
    stock_plans: StockPlans,
) -> Option<Reply> {
    let mut ack = Ack {
        cmd: &msg.cmd,
        code: 0,
        exec_step: None,
        link_id: link_id(),
        msg_id: msg.msg_id.as_deref(),
        plans: None,
        ts: now.timestamp_millis(),
    };
    let topic = channel.sub_topic(config);

    match (channel, msg.cmd.as_str()) {
        (Channel::Ntp, "NTP") => {
            let calibration = clock::calibration(config.plans.as_deref(), stock_plans, msg.ts, now);
            let ntp = ntp_response(config.timezone, now, calibration.tag());
            return Some(Reply::new(topic, &ntp));
        }

        (Channel::System, "BINDING") => {
            // The captured cloud reply, replayed with this exchange's IDs.
            // An installer has none, and a stock fresh from the setup frame
            // needs an answer: until one comes it saves nothing from that
            // frame, goes back to its setup network, and refuses an install
            // (1204, "device not bound").
            let mut payload = match &config.binding_response {
                Some(captured) => captured.clone(),
                None if config.hotspot().is_err() => installer_binding(),
                None => return None,
            };
            payload.insert("ts".into(), now.timestamp_millis().into());
            payload.insert("msgId".into(), msg.msg_id.clone().into());
            payload.insert("linkId".into(), link_id().into());
            return Some(Reply {
                topic,
                payload: Value::Object(payload),
            });
        }

        // Stock erases and rewrites its plan table on any reply with a
        // `plans` key (and takes one unasked), so only a captured table is
        // sent. With none, the request goes unanswered and stock keeps its
        // own; it asks once, without retrying (static analysis of 1.1.15).
        (Channel::Event, "GET_FEEDING_PLAN_EVENT") => ack.plans = Some(config.plans.as_deref()?),
        (Channel::Event, "GRAIN_OUTPUT_EVENT") => ack.exec_step = msg.fields.get("execStep"),
        (
            Channel::Event,
            "DEVICE_START_EVENT" | "ATTR_PUSH_EVENT" | "DEVICE_LOG_REPORT_EVENT" | "ERROR_EVENT",
        ) => {}
        _ => return None,
    }

    Some(Reply::new(topic, &ack))
}

/// Every command the server sends unasked. A closed set with no plan
/// message: stock acts on `GET_FEEDING_PLAN_EVENT` or `FEEDING_PLAN_SERVICE`
/// whether or not it asked, erasing its stored plans, so the server never
/// sends either on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sent {
    /// `ATTR_GET_SERVICE`: stock reports its attributes.
    Attributes,
    /// `OTA_UPGRADE`: the install trial's one request.
    Upgrade,
}

impl Sent {
    pub const fn cmd(self) -> &'static str {
        match self {
            Self::Attributes => "ATTR_GET_SERVICE",
            Self::Upgrade => "OTA_UPGRADE",
        }
    }
}

/// A `service` command without fields of its own.
pub fn service(config: &Config, sent: Sent, id: Uuid, now: DateTime<Utc>) -> Reply {
    command(config, Channel::Service, sent.cmd(), id, now, &Plain {})
}
