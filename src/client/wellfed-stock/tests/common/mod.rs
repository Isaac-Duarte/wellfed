//! Shared fixtures: a synthetic stock configuration, the controller over
//! it, and a server in this process on loopback ports. The tests that run
//! the `wellfed-stock` binary are its crate's (`wellfed-stock-server`).

#![allow(dead_code)]
use wellfed_stock::{
    clock::StockPlans,
    server::{Api, Bind, Options, OtaTrial, Server},
};

use std::{
    net::{Ipv4Addr, SocketAddr, TcpListener},
    path::PathBuf,
    time::Duration,
};

use rumqttc::MqttOptions;
use serde_json::Value;

use wellfed_stock::{build_info::Build, config::Config};

pub use wellfed_stock::controller::Controller;

/// The identity a test build reports.
pub const BUILD: Build = Build {
    commit: "unknown",
    clean: false,
};

pub fn config() -> Config {
    Config {
        version: 1,
        device_id: "AF_TEST_ONLY".to_owned(),
        device_client_id: "AF_TEST_ONLY".to_owned(),
        device_username: "test-device".to_owned(),
        device_password: "test-password".to_owned(),
        server_password: "test-server-password-32-characters".to_owned(),
        api_token: "test-api-token-at-least-32-characters".to_owned(),
        mqtt_listen: "127.0.0.1:18883".parse().unwrap(),
        http_listen: "127.0.0.1:18765".parse().unwrap(),
        timezone: chrono_tz::America::Denver,
        binding_response: None,
        hotspot_interface: Some("wlan1".to_owned()),
        target_mac: Some("02:00:00:00:00:42".to_owned()),
        device_address: None,
    }
}

/// The controller over `path`'s ledger, with [`config`].
pub fn load(path: PathBuf) -> wellfed_stock::Result<Controller> {
    Controller::load(config(), path, BUILD)
}

/// A server in this process, as the app runs one, with
/// the loopback API these tests speak. It alone can enable the trial, which
/// needs [`allow_unreviewed`]. Needs a runtime.
pub fn in_process(path: &std::path::Path, trial: OtaTrial) -> Server {
    Server::start(
        &path.canonicalize().unwrap(),
        Options {
            trial,
            enroll: None,
            api: Api::Loopback,
            bind: Bind::Configured,
            stock_plans: StockPlans::Unknown,
        },
    )
    .unwrap()
}

/// Tests run from uncommitted trees, and production refuses such builds.
/// Call it before any runtime starts.
pub fn allow_unreviewed() {
    // SAFETY: the one test in this binary calls this first, before it starts
    // a thread, so nothing else in the process reads the environment.
    unsafe { std::env::set_var(wellfed_stock::build_info::ALLOW_UNREVIEWED_ENV, "1") };
}

/// The runtime an enabled trial's test runs on: it starts after
/// [`allow_unreviewed`], so it cannot be `#[tokio::test]`'s.
pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

/// `N` distinct free loopback addresses, for the ports a spawned server
/// binds once it has started.
///
/// A port is free when chosen but taken only later, by the server's own
/// bind. One from the kernel's ephemeral range could meanwhile go to any
/// process's port-0 bind or outgoing connection (the server's broker then
/// fails `address in use`), so these are drawn at random from below it,
/// where nothing is handed out unasked. Every listener is held until all
/// are chosen, so the `N` differ.
pub fn addresses<const N: usize>() -> std::io::Result<[SocketAddr; N]> {
    addresses_on(Ipv4Addr::LOCALHOST)
}

/// As [`addresses`], on another loopback `host`.
pub fn addresses_on<const N: usize>(host: Ipv4Addr) -> std::io::Result<[SocketAddr; N]> {
    const LOW: u16 = 10_000;
    let ephemeral = std::fs::read_to_string("/proc/sys/net/ipv4/ip_local_port_range")
        .ok()
        .and_then(|range| range.split_whitespace().next()?.parse::<u16>().ok())
        .unwrap_or(32_768);
    let span = ephemeral
        .checked_sub(LOW)
        .filter(|span| *span > 0)
        .ok_or_else(|| {
            std::io::Error::other(format!(
                "the ephemeral range starts at {ephemeral}: no room below it above {LOW}"
            ))
        })?;

    // std's hasher keys come from the operating system's random source.
    use std::hash::BuildHasher as _;
    let random = std::hash::RandomState::new();
    let mut held = Vec::with_capacity(N);
    let mut draw = 0u64;
    while held.len() < N {
        if draw == 10_000 {
            return Err(std::io::Error::other(format!(
                "no {N} free ports below {ephemeral} on {host}"
            )));
        }
        let offset = u16::try_from(random.hash_one(draw) % u64::from(span)).unwrap();
        if let Ok(listener) = TcpListener::bind((host, LOW + offset)) {
            held.push(listener);
        }
        draw += 1;
    }
    let addresses = held
        .iter()
        .map(TcpListener::local_addr)
        .collect::<std::io::Result<Vec<_>>>()?;
    Ok(addresses
        .try_into()
        .expect("one address for each of the N listeners"))
}

/// Waits up to 10 s: a loaded host (a parallel test binary, a bench update)
/// can delay the spawned server's broker connection past 5 s.
pub async fn ready(config: &Config, client: &reqwest::Client) {
    for _ in 0..200 {
        if let Ok(response) = client
            .get(format!("http://{}/api/status", config.http_listen))
            .bearer_auth(&config.api_token)
            .send()
            .await
            && let Ok(value) = response.json::<Value>().await
            && value["broker_connected"] == true
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("stock server did not become ready");
}

pub async fn status(config: &Config, client: &reqwest::Client) -> Value {
    client
        .get(format!("http://{}/api/status", config.http_listen))
        .bearer_auth(&config.api_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

/// The HTTP client a test reaches the service with, straight to loopback.
pub fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap()
}

/// MQTT options for the configured stock device at the embedded broker,
/// with no credentials yet.
pub fn device_options(config: &Config) -> MqttOptions {
    MqttOptions::new(
        &config.device_client_id,
        config.mqtt_listen.ip().to_string(),
        config.mqtt_listen.port(),
    )
}

/// [`device_options`] with the device's credentials on a clean session.
pub fn device_session(config: &Config) -> MqttOptions {
    let mut options = device_options(config);
    options.set_credentials(&config.device_username, &config.device_password);
    options.set_clean_session(true);
    options
}
