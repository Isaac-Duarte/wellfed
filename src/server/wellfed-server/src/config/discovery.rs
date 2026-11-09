//! The `discovery` setting: how a feeder whose address fails is found.

use serde::Deserialize;

/// Whether a feeder whose stored address fails is looked up over mDNS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Discovery {
    #[default]
    Mdns,
    /// Adopt and reconnect by address only, as on a bridged container
    /// network where multicast does not reach the feeders.
    Off,
}

impl From<Discovery> for wellfed_service::Discovery {
    fn from(discovery: Discovery) -> Self {
        match discovery {
            Discovery::Mdns => Self::Mdns,
            Discovery::Off => Self::Off,
        }
    }
}
