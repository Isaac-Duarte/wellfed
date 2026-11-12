//! What the `serve` binary refuses: it never asks stock to install, and it
//! never starts on a config with a plan table, whose replay would erase
//! stock's plans.

mod common;

use std::process::Command;

use uuid::Uuid;
use wellfed_stock::{ota, storage};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_refuses_trial_publication() {
    let (_dir, path, config) = common::configured();

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

/// Stock replaces its table with whatever a reply carries, so a config with
/// a plan table, even an empty one, is not a config this server reads.
#[test]
fn serve_refuses_a_config_with_a_plan_table() {
    for plans in [serde_json::json!([]), serde_json::json!([{"id": 1}])] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.json");
        let mut config = common::config();
        [config.mqtt_listen, config.http_listen] = common::addresses().unwrap();
        let mut file = serde_json::to_value(&config).unwrap();
        file["plans"] = plans;
        storage::write_json(&path, &file).unwrap();

        let output = Command::new(env!("CARGO_BIN_EXE_wellfed-stock"))
            .args(["--device", "plaf103"])
            .args(["serve", "--config"])
            .arg(&path)
            .output()
            .unwrap();

        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("plans"), "{stderr}");
    }
}
