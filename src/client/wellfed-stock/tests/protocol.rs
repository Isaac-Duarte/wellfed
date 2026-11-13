//! Stock topic parsing and replies: only this device's `/post` topics parse,
//! no reply carries a feed command, clock calibration is withheld while
//! stock may hold plans, and no reply carries plans.

mod common;

use chrono::Utc;
use serde_json::json;
use wellfed_stock::protocol::Channel;
use wellfed_stock::protocol::{self, plaf103};

#[test]
fn messages_from_other_devices_and_retained_topic_shapes_are_rejected() {
    let config = common::config();
    for topic in [
        "dl/PLAF103/ANOTHER/device/event/post",
        "dl/PLAF103/AF_TEST_ONLY/device/event/sub",
        "dl/PLAF103/AF_TEST_ONLY/device/unknown/post",
    ] {
        assert!(protocol::parse(&config, topic, br#"{"cmd":"ATTR_PUSH_EVENT"}"#).is_err());
    }

    assert!(protocol::parse(&config, &config.topic("event/post"), b"[]").is_err());
    assert!(protocol::parse(&config, &config.topic("event/post"), &vec![b'x'; 65_537]).is_err());
}

#[test]
fn acknowledgements_preserve_event_id_and_step_without_sending_a_feed() {
    let config = common::config();
    let input = json!({"cmd":"GRAIN_OUTPUT_EVENT","msgId":"event42","execStep":"GRAIN_END"});
    let (channel, msg) = protocol::parse(
        &config,
        &config.topic("event/post"),
        &serde_json::to_vec(&input).unwrap(),
    )
    .unwrap();

    let reply = plaf103::reply(
        &config,
        channel,
        &msg,
        Utc::now(),
        wellfed_stock::clock::StockPlans::Unknown,
    )
    .unwrap();
    assert_eq!(reply.topic, config.topic("event/sub"));
    assert_eq!(reply.payload["msgId"], "event42");
    assert_eq!(reply.payload["execStep"], "GRAIN_END");
    assert_eq!(reply.payload["code"], 0);
    assert!(reply.payload.get("grainNum").is_none());
}

#[test]
fn stock_ota_report_topic_is_parsed_without_sending_an_acknowledgement_or_command() {
    let config = common::config();
    for cmd in ["OTA_UPGRADE", "OTA_PROGRESS", "OTA_INFORM"] {
        let payload =
            serde_json::to_vec(&json!({"cmd":cmd,"msgId":"trial","progress":40})).unwrap();
        let (channel, msg) = protocol::parse(&config, &config.topic("ota/post"), &payload).unwrap();
        assert_eq!(channel, Channel::Ota);
        assert_eq!(msg.cmd, cmd);
        assert!(
            plaf103::reply(
                &config,
                channel,
                &msg,
                Utc::now(),
                wellfed_stock::clock::StockPlans::Unknown
            )
            .is_none()
        );
        assert!(protocol::parse(&config, "dl/PLAF103/ANOTHER/device/ota/post", &payload).is_err());
    }
}

#[test]
fn configuration_reply_uses_its_own_channel_without_an_automatic_response() {
    let config = common::config();
    let (channel, msg) = protocol::parse(
        &config,
        &config.topic("config/post"),
        br#"{"cmd":"GET_CONFIG","msgId":"device-generated"}"#,
    )
    .unwrap();

    assert_eq!(channel, Channel::Config);
    assert!(
        plaf103::reply(
            &config,
            channel,
            &msg,
            Utc::now(),
            wellfed_stock::clock::StockPlans::Unknown
        )
        .is_none()
    );
    assert!(
        protocol::parse(
            &config,
            "dl/PLAF103/ANOTHER/device/config/post",
            br#"{"cmd":"GET_CONFIG"}"#
        )
        .is_err()
    );
}

/// Stock erases and rewrites its plan table on any message with a `plans`
/// key, so the plan request goes unanswered and no reply carries the key.
#[test]
fn plans_are_never_sent() {
    let config = common::config();
    let ask = |config: &wellfed_stock::config::Config, channel: Channel, topic: &str, cmd: &str| {
        let body = json!({"cmd": cmd, "msgId": "m", "ts": 0, "execStep": "GRAIN_END"});
        let (parsed, msg) = protocol::parse(
            config,
            &config.topic(topic),
            &serde_json::to_vec(&body).unwrap(),
        )
        .unwrap();
        assert_eq!(parsed, channel);
        plaf103::reply(
            config,
            channel,
            &msg,
            Utc::now(),
            wellfed_stock::clock::StockPlans::Unknown,
        )
    };

    assert!(
        ask(
            &config,
            Channel::Event,
            "event/post",
            "GET_FEEDING_PLAN_EVENT"
        )
        .is_none()
    );
    for (channel, topic, cmd) in [
        (Channel::Ntp, "ntp/post", "NTP"),
        (Channel::Event, "event/post", "GRAIN_OUTPUT_EVENT"),
        (Channel::Event, "event/post", "DEVICE_START_EVENT"),
        (Channel::Event, "event/post", "ATTR_PUSH_EVENT"),
        (Channel::Event, "event/post", "DEVICE_LOG_REPORT_EVENT"),
        (Channel::Event, "event/post", "ERROR_EVENT"),
    ] {
        let reply = ask(&config, channel, topic, cmd).expect("acknowledged");
        assert!(reply.payload.get("plans").is_none(), "{cmd}");
    }
}

/// The clock turns on what is known of stock's own plans: set after the
/// frame, left as it reads otherwise.
#[test]
fn an_installer_sets_the_clock_only_after_the_frame() {
    let config = common::config();
    let (_, msg) = protocol::parse(
        &config,
        &config.topic("ntp/post"),
        br#"{"cmd":"NTP","ts":1000}"#,
    )
    .unwrap();
    let tag = |plans| {
        plaf103::reply(&config, Channel::Ntp, &msg, Utc::now(), plans)
            .unwrap()
            .payload["calibrationTag"]
            .clone()
    };

    assert_eq!(tag(wellfed_stock::clock::StockPlans::Erased), true);
    assert_eq!(tag(wellfed_stock::clock::StockPlans::Unknown), false);
}
