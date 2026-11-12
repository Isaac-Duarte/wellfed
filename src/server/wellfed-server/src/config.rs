//! The server's settings: `server.toml` in the data directory, under the
//! flags and environment variables that override it.
//!
//! The file holds what is neither a secret nor a household setting: the
//! sealing key has its own file ([`crate::key`]) and the household's zone
//! lives in the database. Every setting has a default, so a server with no
//! file (or an empty one) listens on localhost:8080 with discovery and
//! advertisement on.
//!
//! A flag, or the environment variable it names, replaces the file's value
//! whole: `--host` given once replaces the file's `hosts` rather than adding
//! to them. Values are checked while the file is read, so a bad one is
//! refused with its key's path and its line, not later by whatever uses it.

use std::net::{Ipv4Addr, SocketAddr};
use std::num::NonZeroU32;
use std::path::Path;
use std::str::FromStr;

use serde::Deserialize;

pub use crate::proxy::ProxyRange;

mod advertise;
mod discovery;
mod error;
mod releases;
mod webhook;

pub use advertise::{Advertise, AdvertiseOverrides};
pub use discovery::Discovery;
pub use error::{ConfigError, InvalidValue, Refused};
pub use releases::{RELEASES, ReleaseOverrides, Releases, UpdateKey};
pub use webhook::{Webhook, WebhookError, WebhookOverrides, WebhookUrl};

use releases::ReleasesFile;
use webhook::WebhookFile;

/// The settings file, in the data directory.
pub const FILE: &str = "server.toml";

/// The database, in the data directory.
pub const DATABASE: &str = "server.db";

/// The database's backups, in the data directory.
pub const BACKUPS: &str = "backups";

/// The feeders' stock backups, one directory per feeder identity, in the
/// data directory.
pub const STOCK: &str = "stock";

/// The label the server enrolls under on each feeder, unless set.
const LABEL: &str = "wellfed-server";

/// How many backups are kept, unless set.
const BACKUPS_KEPT: NonZeroU32 = NonZeroU32::MIN.saturating_add(6);

/// The settings the server runs with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerConfig {
    /// Where the HTTP API listens.
    pub listen: SocketAddr,
    /// Where the same API listens over HTTPS, with the certificate the
    /// server's identity key signs: the address the app reaches it by on
    /// the home network, and the one mDNS announces. None by default.
    pub tls_listen: Option<SocketAddr>,
    /// Names the household reaches the server by, besides IP addresses and
    /// `localhost`, which always work.
    pub hosts: Vec<HostName>,
    /// The label the server enrolls under on each feeder.
    pub label: Label,
    pub discovery: Discovery,
    /// Whether the server announces itself over mDNS.
    pub advertise: Advertise,
    /// Reverse proxies whose forwarded headers are believed: addresses or
    /// CIDR ranges. None by default: a request's scheme and address are
    /// its own connection's.
    pub trusted_proxies: Vec<ProxyRange>,
    pub retention: Retention,
    pub releases: Releases,
    pub backups: Backups,
    pub webhook: Webhook,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Retention {
    /// How many days raw samples are kept; forever when `None`. Hourly
    /// rollups are always kept.
    pub raw_sample_days: Option<NonZeroU32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backups {
    /// How many backups are kept, the daily ones and those asked for
    /// together.
    pub keep: NonZeroU32,
}

impl Default for Backups {
    fn default() -> Self {
        Self { keep: BACKUPS_KEPT }
    }
}

/// The label the server enrolls under on a feeder: 1-16 bytes with no NUL,
/// the most the feeder's client table keeps.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct Label(String);

impl Label {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<Label> for String {
    fn from(label: Label) -> Self {
        label.0
    }
}

impl TryFrom<String> for Label {
    type Error = InvalidValue;

    fn try_from(label: String) -> Result<Self, InvalidValue> {
        let bytes = label.as_bytes();
        if bytes.is_empty() || bytes.len() > 16 || bytes.contains(&0) {
            return Err(InvalidValue::Label { len: bytes.len() });
        }
        Ok(Self(label))
    }
}

impl FromStr for Label {
    type Err = InvalidValue;

    fn from_str(label: &str) -> Result<Self, InvalidValue> {
        Self::try_from(label.to_owned())
    }
}

/// A DNS name the server answers to, lowercased.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct HostName(String);

impl HostName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<HostName> for String {
    fn from(name: HostName) -> Self {
        name.0
    }
}

impl TryFrom<String> for HostName {
    type Error = InvalidValue;

    fn try_from(name: String) -> Result<Self, InvalidValue> {
        // RFC 1123 labels: 1-63 letters, digits and inner hyphens.
        let labels_ok = name.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        });
        if !labels_ok || name.len() > 253 {
            return Err(InvalidValue::HostName { name });
        }
        Ok(Self(name.to_ascii_lowercase()))
    }
}

impl FromStr for HostName {
    type Err = InvalidValue;

    fn from_str(name: &str) -> Result<Self, InvalidValue> {
        Self::try_from(name.to_owned())
    }
}

/// The flags, each with its environment variable, that override the file.
/// An empty list means the flag was not given.
#[derive(Debug, Default, clap::Args)]
pub struct Overrides {
    /// Where the HTTP API listens (`listen`; default 127.0.0.1:8080): this
    /// computer only, unless the household's other devices should reach it
    /// (`0.0.0.0:8080`).
    #[arg(long, env = "WELLFED_SERVER_LISTEN")]
    pub listen: Option<SocketAddr>,
    /// Where the API also listens over HTTPS, with the certificate the
    /// server's identity key signs (`tls_listen`; default none), such as
    /// `0.0.0.0:8443`: how the app reaches the server on the home network,
    /// and the address mDNS announces.
    #[arg(long, env = "WELLFED_SERVER_TLS_LISTEN")]
    pub tls_listen: Option<SocketAddr>,
    /// A name the household reaches the server by (`hosts`), such as
    /// `feeder.lan`; IP addresses and `localhost` always work. Repeat for
    /// more names.
    #[arg(
        long = "host",
        env = "WELLFED_SERVER_HOSTS",
        value_delimiter = ',',
        value_name = "NAME"
    )]
    pub hosts: Vec<HostName>,
    /// The label the server enrolls under on each feeder (`label`).
    #[arg(long, env = "WELLFED_SERVER_LABEL")]
    pub label: Option<Label>,
    /// Whether a feeder whose address fails is looked up over mDNS
    /// (`discovery`; default mdns).
    #[arg(long, env = "WELLFED_SERVER_DISCOVERY")]
    pub discovery: Option<Discovery>,
    /// Never look feeders up over mDNS: `--discovery off`, over
    /// `--discovery` and its variable. Not a conflict with them, since clap
    /// counts a set variable as given and would refuse the pair.
    #[arg(long)]
    pub no_discovery: bool,
    #[command(flatten)]
    pub advertise: AdvertiseOverrides,
    /// A reverse proxy whose forwarded headers are believed, as an address
    /// or a CIDR range (`trusted_proxies`). Repeat for more.
    #[arg(
        long = "trusted-proxy",
        env = "WELLFED_SERVER_TRUSTED_PROXIES",
        value_delimiter = ',',
        value_name = "ADDRESS[/BITS]"
    )]
    pub trusted_proxies: Vec<ProxyRange>,
    /// Days raw samples are kept (`retention.raw_sample_days`; default
    /// forever).
    #[arg(long, env = "WELLFED_SERVER_RAW_SAMPLE_DAYS")]
    pub raw_sample_days: Option<NonZeroU32>,
    #[command(flatten)]
    pub releases: ReleaseOverrides,
    /// Backups kept (`backups.keep`; default 7).
    #[arg(long, env = "WELLFED_SERVER_BACKUPS_KEEP")]
    pub backups_keep: Option<NonZeroU32>,
    #[command(flatten)]
    pub webhook: WebhookOverrides,
}

/// `server.toml` as written: every key optional, none unknown.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    listen: Option<SocketAddr>,
    tls_listen: Option<SocketAddr>,
    hosts: Option<Vec<HostName>>,
    label: Option<Label>,
    discovery: Option<Discovery>,
    advertise: Option<Advertise>,
    trusted_proxies: Option<Vec<ProxyRange>>,
    #[serde(default)]
    retention: RetentionFile,
    #[serde(default)]
    releases: ReleasesFile,
    #[serde(default)]
    backups: BackupsFile,
    #[serde(default)]
    webhook: WebhookFile,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetentionFile {
    raw_sample_days: Option<NonZeroU32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupsFile {
    keep: Option<NonZeroU32>,
}

impl ServerConfig {
    /// The settings from `data`'s `server.toml` (the defaults when there is
    /// none) under `overrides`.
    pub fn load(data: &Path, overrides: Overrides) -> Result<Self, ConfigError> {
        let path = data.join(FILE);
        let file = match std::fs::read_to_string(&path) {
            Ok(text) => parse(&path, &text)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => File::default(),
            Err(source) => return Err(ConfigError::Read { path, source }),
        };
        Self::merge(file, overrides).checked(&path)
    }

    /// The settings `text` gives, under no overrides; `path` names it in an
    /// error.
    pub fn parse(path: &Path, text: &str) -> Result<Self, ConfigError> {
        Self::merge(parse(path, text)?, Overrides::default()).checked(path)
    }

    /// These settings, once the rules across their keys hold.
    fn checked(self, path: &Path) -> Result<Self, ConfigError> {
        let path = path.to_owned();
        let webhook = self.webhook.target();
        webhook.map_err(|source| ConfigError::Webhook { path, source })?;
        Ok(self)
    }

    fn merge(file: File, overrides: Overrides) -> Self {
        let discovery = if overrides.no_discovery {
            Some(Discovery::Off)
        } else {
            overrides.discovery
        };

        Self {
            listen: overrides
                .listen
                .or(file.listen)
                .unwrap_or(SocketAddr::from((Ipv4Addr::LOCALHOST, 8080))),
            tls_listen: overrides.tls_listen.or(file.tls_listen),
            hosts: given(overrides.hosts).or(file.hosts).unwrap_or_default(),
            label: overrides
                .label
                .or(file.label)
                .unwrap_or_else(|| Label(LABEL.to_owned())),
            discovery: discovery.or(file.discovery).unwrap_or_default(),
            advertise: overrides.advertise.merge(file.advertise),
            trusted_proxies: given(overrides.trusted_proxies)
                .or(file.trusted_proxies)
                .unwrap_or_default(),
            retention: Retention {
                raw_sample_days: overrides.raw_sample_days.or(file.retention.raw_sample_days),
            },
            releases: Releases::merge(file.releases, overrides.releases),
            backups: Backups {
                keep: overrides
                    .backups_keep
                    .or(file.backups.keep)
                    .unwrap_or(BACKUPS_KEPT),
            },
            webhook: Webhook::merge(file.webhook, overrides.webhook),
        }
    }
}

/// A list flag counts as given when it names at least one value.
fn given<T>(values: Vec<T>) -> Option<Vec<T>> {
    (!values.is_empty()).then_some(values)
}

fn parse(path: &Path, text: &str) -> Result<File, ConfigError> {
    let deserializer = toml::Deserializer::parse(text).map_err(|error| {
        let mut source = Refused::new(text, &error, false);
        if source.in_webhook(text) {
            source.reason = None;
        }
        ConfigError::Syntax {
            path: path.to_owned(),
            source,
        }
    })?;

    serde_path_to_error::deserialize(deserializer).map_err(|error| {
        let field = error.path().to_string();
        let secret = field == "webhook" || field.starts_with("webhook.");
        let source = Refused::new(text, error.inner(), secret);
        ConfigError::Invalid {
            path: path.to_owned(),
            field,
            source,
        }
    })
}
