//! What the `serve` binary refuses: it never asks stock to install, and it
//! never starts on a plan snapshot, whose replay would erase stock's plans.

mod common;

use std::process::Command;

use uuid::Uuid;
use wellfed_stock::{ota, storage};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_refuses_trial_publication() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.json");
    let mut config = common::config();
    config.plans = None;
    [config.mqtt_listen, config.http_listen] = common::addresses().unwrap();
    storage::write_json(&path, &config).unwrap();

    let service = common::start(&path);
    let http = common::http();
    common::ready(&config, &http).await;

    let response = http
        .post(format!("http://{}/api/ota/trial", config.http_listen))
        .bearer_auth(&config.api_token)
        .json(&ota::trial::Input {
            request_id: Uuid::new_v4(),
            server_token: Uuid::new_v4(),
        })
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    let status = common::status(&config, &http).await;
    assert_eq!(status["ota_trial_enabled"], false);
    assert!(status["ota_trials"].as_array().unwrap().is_empty());
    drop(service);
}

/// Stock replaces its table with whatever a reply carries, so an empty
/// snapshot is refused as surely as a full one.
#[test]
fn serve_refuses_a_plan_snapshot() {
    for plans in [vec![], vec![serde_json::json!({"id": 1})]] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.json");
        let mut config = common::config();
        config.plans = Some(plans);
        [config.mqtt_listen, config.http_listen] = common::addresses().unwrap();
        storage::write_json(&path, &config).unwrap();

        let output = Command::new(env!("CARGO_BIN_EXE_wellfed-stock"))
            .args(["--device", "plaf103"])
            .args(["serve", "--config"])
            .arg(&path)
            .output()
            .unwrap();

        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("holds a plan snapshot"), "{stderr}");
    }
}
