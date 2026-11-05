//! The stock server's private configuration (`private/installer.json`, or a
//! captured file named by `--config`; format version 1): stock's captured
//! device credentials and plans, if any, and the server's own secrets and
//! addresses.
//!
//! It holds working device credentials, so it is written mode 0600 and has
//! no `Debug`. Edit it only while the service is
//! stopped. `validate` keeps the HTTP API on loopback and the broker on one
//! specific IPv4 address.
//!
//! An installer's config ([`InstallConfig`], written by `init-installer`)
//! has no device at all: the broker trusts the first stock feeder that
//! connects during an install ([`crate::enroll`]), and [`load`] then takes
//! the device from that enrollment beside the config. A file names its
//! device one way or the other, never both.

use std::{
    fs,
    net::{IpAddr, SocketAddr},
    path::Path,
};

use anyhow::{Context, Result, bail, ensure};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::protocol::{self, plaf103};
use crate::{enroll::Enrollment, storage};

/// The field a config with a captured device has, and an installer's lacks.
const DEVICE_FIELD: &str = "device_id";

// Intentionally no Debug: this structure contains working device credentials.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub device_id: String,
    pub device_client_id: String,
    pub device_username: String,
    pub device_password: String,

    pub server_password: String,
    pub api_token: String,

    pub mqtt_listen: SocketAddr,
    pub http_listen: SocketAddr,

    pub timezone: Tz,
    /// Stock's plan table as a capture recorded the cloud sending it,
    /// replayed verbatim to `GET_FEEDING_PLAN_EVENT`. `None` when nothing
    /// was captured (an installer's config): the request then goes
    /// unanswered, since stock erases and rewrites its stored table on any
    /// reply with a `plans` key, and an empty list would erase the owner's
    /// plans before the installer imports them. `Some([])` is a captured
    /// empty table, answered as captured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plans: Option<Vec<Value>>,
    /// The captured cloud `BINDING` reply, replayed verbatim.
    #[serde(default)]
    pub binding_response: Option<Map<String, Value>>,

    /// The bench hotspot the feeder joins, and the feeder's MAC on it: what
    /// neighbour discovery needs. An installer's config has neither.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hotspot_interface: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_mac: Option<String>,
    /// Where an enrolled feeder first connected from, which the image
    /// server admits when no MAC is known. Filled from the enrollment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_address: Option<IpAddr>,
}

/// Stock's device ID: what its topics carry, and (physical, the captured
/// CONNECT) its MQTT client ID too.
pub fn valid_device_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The server's own checks, shared by both kinds of config.
fn validate_server(
    version: u32,
    http_listen: SocketAddr,
    mqtt_listen: SocketAddr,
    api_token: &str,
    server_password: &str,
) -> Result<()> {
    ensure!(version == 1, "unsupported configuration version");
    ensure!(
        http_listen.ip().is_loopback(),
        "HTTP API must bind to loopback"
    );
    ensure!(
        mqtt_listen.is_ipv4() && !mqtt_listen.ip().is_unspecified(),
        "MQTT must bind to a specific IPv4 address"
    );
    ensure!(
        api_token.len() >= 32 && server_password.len() >= 32,
        "server secrets must have at least 32 characters"
    );
    Ok(())
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        validate_server(
            self.version,
            self.http_listen,
            self.mqtt_listen,
            &self.api_token,
            &self.server_password,
        )?;
        ensure!(
            valid_device_id(&self.device_id),
            "invalid device identifier"
        );
        ensure!(
            !self.device_password.is_empty() && !self.device_username.is_empty(),
            "device credentials missing"
        );

        if let Some(interface) = &self.hotspot_interface {
            ensure!(
                interface
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                    && !interface.is_empty(),
                "invalid hotspot interface"
            );
        }
        if let Some(mac) = &self.target_mac {
            ensure!(
                mac.split(':').count() == 6
                    && mac
                        .split(':')
                        .all(|part| part.len() == 2 && u8::from_str_radix(part, 16).is_ok()),
                "invalid target MAC"
            );
        }

        Ok(())
    }

    /// The hotspot interface and the feeder's MAC on it, which only a
    /// bench config has.
    pub fn hotspot(&self) -> Result<(&str, &str)> {
        match (&self.hotspot_interface, &self.target_mac) {
            (Some(interface), Some(mac)) => Ok((interface, mac)),
            _ => bail!("this config names no hotspot interface and feeder MAC (an installer's)"),
        }
    }

    pub fn topic(&self, suffix: &str) -> String {
        protocol::device_topic(plaf103::PID, &self.device_id, suffix)
    }

    pub fn server_client_id(&self) -> String {
        server_client_id(&self.device_id)
    }
}

/// The server's own MQTT client ID for `device_id`'s session.
pub fn server_client_id(device_id: &str) -> String {
    format!("local-server-{device_id}")
}

/// An installer's config: the server's secrets and addresses, and no
/// device until one enrolls. Like [`Config`], no `Debug`.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InstallConfig {
    pub version: u32,
    pub server_password: String,
    pub api_token: String,
    pub mqtt_listen: SocketAddr,
    pub http_listen: SocketAddr,
    pub timezone: Tz,
}

impl InstallConfig {
    pub fn validate(&self) -> Result<()> {
        validate_server(
            self.version,
            self.http_listen,
            self.mqtt_listen,
            &self.api_token,
            &self.server_password,
        )
    }
}

/// A new installer's config at `output`, with fresh secrets; never replaces
/// a file.
pub fn init_installer(output: &Path, mqtt_listen: SocketAddr, timezone: Tz) -> Result<()> {
    let config = InstallConfig {
        version: 1,
        server_password: Uuid::new_v4().simple().to_string(),
        api_token: Uuid::new_v4().simple().to_string(),
        mqtt_listen,
        http_listen: "127.0.0.1:8765".parse()?,
        timezone,
    };
    config.validate()?;
    storage::write_new_json(output, &config)
}

/// What a config file names as its device.
pub enum State {
    /// A captured device, or an installer's with its enrollment.
    Device(Box<Config>),
    /// An installer's config before any feeder enrolled.
    Unenrolled(InstallConfig),
}

/// The config at `path` with its device: captured, or enrolled beside it.
pub fn load(path: &Path) -> Result<Config> {
    match state(path)? {
        State::Device(config) => Ok(*config),
        State::Unenrolled(_) => bail!(
            "{}: no feeder has enrolled yet; start serve with --enroll-minutes during the install",
            path.display()
        ),
    }
}

/// The config at `path`, and whether it has a device yet.
pub fn state(path: &Path) -> Result<State> {
    let bytes = fs::read(path).context("read configuration")?;
    let record = Enrollment::path(path);
    let enrollment = Enrollment::load(&record)
        .with_context(|| format!("invalid enrollment {}", record.display()))?;
    let state = parse_state(&bytes, enrollment).context("invalid server configuration")?;
    match &state {
        State::Device(config) => config.validate()?,
        State::Unenrolled(config) => config.validate()?,
    }
    Ok(state)
}

/// A config's fields with the device taken from `enrollment` when the file
/// has none of its own; both at once is refused, since which feeder the
/// server answers would then be a guess.
fn parse_state(bytes: &[u8], enrollment: Option<Enrollment>) -> Result<State> {
    let UniqueKeys(mut fields) = serde_json::from_slice(bytes)?;

    match (fields.contains_key(DEVICE_FIELD), enrollment) {
        (true, None) => Ok(State::Device(Box::new(serde_json::from_value(
            Value::Object(fields),
        )?))),
        (true, Some(_)) => {
            bail!("the config names a captured device and a feeder enrolled beside it; keep one")
        }
        (false, None) => Ok(State::Unenrolled(serde_json::from_value(Value::Object(
            fields,
        ))?)),
        (false, Some(enrolled)) => {
            for (key, value) in [
                (DEVICE_FIELD, &enrolled.client_id),
                ("device_client_id", &enrolled.client_id),
                ("device_username", &enrolled.username),
                ("device_password", &enrolled.password),
            ] {
                fields.insert(key.into(), Value::String(value.clone()));
            }
            fields.insert(
                "device_address".into(),
                Value::String(enrolled.address.to_string()),
            );
            Ok(State::Device(Box::new(serde_json::from_value(
                Value::Object(fields),
            )?)))
        }
    }
}

/// A JSON object whose keys each appear once.
struct UniqueKeys(Map<String, Value>);

impl<'de> Deserialize<'de> for UniqueKeys {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = UniqueKeys;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a JSON object")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut access: A,
            ) -> Result<UniqueKeys, A::Error> {
                let mut fields = Map::new();
                while let Some((key, value)) = access.next_entry::<String, Value>()? {
                    if fields.contains_key(&key) {
                        return Err(serde::de::Error::custom(format!("duplicate field `{key}`")));
                    }
                    fields.insert(key, value);
                }
                Ok(UniqueKeys(fields))
            }
        }

        deserializer.deserialize_map(Visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STOCK: &str = r#""version":1,"device_id":"AF_TEST","device_client_id":"AF_TEST",
        "device_username":"dev","device_password":"pw",
        "server_password":"ssssssssssssssssssssssssssssssss",
        "api_token":"tttttttttttttttttttttttttttttttt",
        "mqtt_listen":"127.0.0.1:1883","http_listen":"127.0.0.1:8765",
        "timezone":"UTC","plans":[],"binding_response":null,
        "hotspot_interface":"wlan1","target_mac":"02:00:00:00:00:01""#;

    const INSTALLER: &str = r#"{"version":1,
        "server_password":"ssssssssssssssssssssssssssssssss",
        "api_token":"tttttttttttttttttttttttttttttttt",
        "mqtt_listen":"127.0.0.1:1883","http_listen":"127.0.0.1:8765",
        "timezone":"UTC"}"#;

    /// A config with its own device.
    fn parse(bytes: &[u8]) -> Result<Config> {
        match parse_state(bytes, None)? {
            State::Device(config) => Ok(*config),
            State::Unenrolled(_) => bail!("no device"),
        }
    }

    fn enrolled() -> Enrollment {
        Enrollment {
            client_id: "AF_NEW".into(),
            username: "stock".into(),
            password: "its-own".into(),
            address: "192.168.4.20".parse().unwrap(),
            enrolled_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn an_installer_s_config_takes_its_device_from_the_enrollment() {
        let State::Unenrolled(config) = parse_state(INSTALLER.as_bytes(), None).unwrap() else {
            panic!("no device before an enrollment");
        };
        assert!(config.validate().is_ok());

        let State::Device(config) = parse_state(INSTALLER.as_bytes(), Some(enrolled())).unwrap()
        else {
            panic!("the enrolled device");
        };
        assert!(config.validate().is_ok());
        assert_eq!(config.device_id, "AF_NEW");
        assert_eq!(config.device_client_id, "AF_NEW");
        assert_eq!(config.device_password, "its-own");
        assert_eq!(config.device_address, Some("192.168.4.20".parse().unwrap()));
        assert!(config.hotspot().is_err(), "no redirect for an installer");
        assert_eq!(
            config.plans, None,
            "no snapshot: stock's plans are left alone"
        );
    }

    #[test]
    fn a_captured_device_and_an_enrollment_together_are_refused() {
        let captured = format!("{{{STOCK}}}");
        assert!(parse_state(captured.as_bytes(), Some(enrolled())).is_err());
        let config = parse(captured.as_bytes()).unwrap();
        assert_eq!(config.hotspot().unwrap(), ("wlan1", "02:00:00:00:00:01"));
    }

    #[test]
    fn a_repeated_key_is_refused_as_the_struct_parse_did() {
        let version_twice = format!("{{{STOCK},\"version\":1}}");
        let error = parse(version_twice.as_bytes()).err().unwrap();
        assert!(error.to_string().contains("duplicate field `version`"));
    }

    #[test]
    fn any_other_unknown_field_is_still_refused() {
        let extra = format!("{{{STOCK},\"other\":1}}");
        assert!(parse(extra.as_bytes()).is_err());
    }
}
