//! `init-installer`: what it writes, and what it refuses to write. And what
//! a config read back may name.

mod common;

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;
use wellfed_stock::{
    config::{self, State},
    storage,
};

fn init(listen: &str, output: &Path, zone: Option<&str>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_wellfed-stock"));
    // The stock model whose message set every action speaks.
    command.args(["--device", "plaf103"]);
    command
        .args(["init-installer", "--mqtt-listen", listen, "--output"])
        .arg(output);
    if let Some(zone) = zone {
        command.args(["--timezone", zone]);
    }
    command.output().unwrap()
}

#[test]
fn writes_settings_with_utc_default_and_private_permissions() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("installer.json");

    let result = init("127.0.0.1:18883", &path, None);
    assert!(result.status.success(), "{:?}", result.stderr);

    let State::Unenrolled(config) = config::state(&path).unwrap() else {
        panic!("an installer's config names no device");
    };
    assert_eq!(config.timezone, chrono_tz::UTC);
    assert_eq!(config.mqtt_listen.to_string(), "127.0.0.1:18883");
    assert_ne!(config.api_token, config.server_password);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn takes_an_iana_timezone_and_refuses_to_replace_a_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("installer.json");

    assert!(
        init("127.0.0.1:18883", &path, Some("Europe/Lisbon"))
            .status
            .success()
    );
    let State::Unenrolled(config) = config::state(&path).unwrap() else {
        panic!("an installer's config names no device");
    };
    assert_eq!(config.timezone, chrono_tz::Europe::Lisbon);
    let before = fs::read(&path).unwrap();

    assert!(!init("127.0.0.1:18884", &path, None).status.success());
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn a_wildcard_or_ipv6_broker_bind_writes_nothing() {
    for listen in ["0.0.0.0:1883", "[::1]:1883", "[::]:1883", "not-an-address"] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("installer.json");

        assert!(!init(listen, &path, None).status.success(), "{listen}");
        assert!(!path.exists(), "{listen}");
    }
}

/// A config may name only what an interface can be called.
#[test]
fn a_hotspot_interface_with_other_characters_is_refused() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("server.json");
    let mut written = common::config();

    written.hotspot_interface = Some("wlan-1_a".to_owned());
    storage::write_json(&path, &written).unwrap();
    assert!(config::state(&path).is_ok());

    for name in ["", "wlan 1", "wlan1;reboot", "../wlan1"] {
        written.hotspot_interface = Some(name.to_owned());
        storage::write_json(&path, &written).unwrap();

        let Err(error) = config::state(&path) else {
            panic!("{name:?} was accepted");
        };
        assert!(
            format!("{error:#}").contains("invalid hotspot interface"),
            "{name:?}: {error:#}"
        );
    }
}
