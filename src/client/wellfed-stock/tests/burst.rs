//! A device that bursts faster than the server's client drains its queue
//! costs at most the replies it gets: the server keeps running and its
//! status keeps answering.

mod common;

use chrono::Utc;
use rumqttc::{AsyncClient, QoS};
use serde_json::json;
use std::time::Duration;
use wellfed_stock::server::OtaTrial;
use wellfed_stock::storage;

#[test]
fn a_device_burst_never_ends_the_server() {
    common::allow_unreviewed();
    common::runtime().block_on(burst());
}

async fn burst() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.json");
    let mut config = common::config();
    [config.mqtt_listen, config.http_listen] = common::addresses().unwrap();
    storage::write_json(&path, &config).unwrap();

    let service = common::in_process(&path, OtaTrial::Disabled);
    let http = common::http();
    common::ready(&config, &http).await;

    let (client, mut events) = AsyncClient::new(common::device_session(&config), 1024);
    let reader = tokio::spawn(async move {
        loop {
            if let Err(error) = events.poll().await {
                return error;
            }
        }
    });

    // Each start event earns a reply and an attribute query: two server
    // publishes for one device publish.
    let start = json!({"cmd":"DEVICE_START_EVENT","ts":Utc::now().timestamp_millis(),
        "softwareVersion":"1.1.15","powerType":1})
    .to_string();
    for _ in 0..3000 {
        client
            .publish(
                config.topic("event/post"),
                QoS::AtMostOnce,
                false,
                start.clone(),
            )
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_secs(2)).await;

    assert!(!service.is_finished(), "the burst ended the stock server");
    assert_eq!(
        common::status(&config, &http).await["broker_connected"],
        true
    );
    reader.abort();
}
