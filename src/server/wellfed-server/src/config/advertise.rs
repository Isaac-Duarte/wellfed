//! The `advertise` setting: whether the server announces itself on the LAN
//! over mDNS, so the app finds it without a typed address.
//!
//! It is apart from `discovery`, which is about finding the feeders: a
//! server whose feeders have fixed addresses may still be announced, and
//! one the household reaches only through a proxy may still look its
//! feeders up.

use serde::Deserialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Advertise {
    /// Announce `_wellfed-server._tcp` while the server listens beyond
    /// this computer. A server on a loopback address is never announced:
    /// nothing else could reach it.
    #[default]
    Mdns,
    /// Never announce: the app is given the server's address.
    Off,
}

/// The flags, each with its environment variable, that override the file's
/// `advertise`.
#[derive(Debug, Default, clap::Args)]
pub struct AdvertiseOverrides {
    /// Whether the server announces itself on the LAN over mDNS, while it
    /// listens beyond this computer (`advertise`; default mdns).
    #[arg(long, env = "WELLFED_SERVER_ADVERTISE")]
    pub advertise: Option<Advertise>,
    /// Never announce the server over mDNS: `--advertise off`, over
    /// `--advertise` and its variable, as `--no-discovery` is.
    #[arg(long)]
    pub no_advertise: bool,
}

impl AdvertiseOverrides {
    pub(super) fn merge(self, file: Option<Advertise>) -> Advertise {
        if self.no_advertise {
            return Advertise::Off;
        }
        self.advertise.or(file).unwrap_or_default()
    }
}
