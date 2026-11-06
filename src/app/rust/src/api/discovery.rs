//! Feeders on the LAN, for the desktops. Android browses through
//! NsdManager, which holds the multicast lock the phone needs. `nsd` has no
//! Linux side, and macOS browses here too, so both desktops find feeders
//! through the same `wellfed-connect` the CLI locates them with.

use std::sync::Arc;
use std::time::Duration;

use flutter_rust_bridge::frb;
use wellfed_connect::{Announced, Browse, Browsed};

use crate::api::error::BridgeError;
use crate::frb_generated::StreamSink;

/// How often a quiet browse checks that someone still listens.
const LISTENING: Duration = Duration::from_secs(1);

/// A feeder answering on the LAN.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundFeeder {
    /// Its identity key, hex, from the TXT `id`.
    pub identity: String,
    /// `ip:port`, IPv4: where its device API listens.
    pub address: String,
    /// The service instance name the feeder announces.
    pub name: String,
}

impl From<Announced> for FoundFeeder {
    fn from(feeder: Announced) -> Self {
        Self {
            identity: feeder.identity,
            address: feeder.address.to_string(),
            name: feeder.name,
        }
    }
}

/// One browse for feeders, from [`Self::run`] until [`Self::stop`].
#[frb(opaque)]
pub struct FeederBrowse {
    browse: Arc<Browse>,
}

impl FeederBrowse {
    /// Start the mDNS daemon; nothing is heard until [`Self::run`].
    #[frb(sync)]
    pub fn new() -> Result<Self, BridgeError> {
        Ok(Self {
            browse: Arc::new(Browse::start()?),
        })
    }

    /// Every feeder heard, again each time it re-announces, until the browse
    /// stops or nobody listens. The stream closes when it ends.
    pub fn run(&self, found: StreamSink<FoundFeeder>) {
        let browse = self.browse.clone();

        // A browse blocks its thread between answers. With no thread the sink
        // drops with the closure, which closes Dart's stream, and the screen
        // reads that as a browse that heard nothing.
        let _ = std::thread::Builder::new()
            .name("wellfed-browse".into())
            .spawn(move || {
                loop {
                    match browse.next(LISTENING) {
                        Browsed::Found(feeder) => {
                            if found.add(feeder.into()).is_err() {
                                break;
                            }
                        }
                        Browsed::Quiet => {}
                        Browsed::Stopped => break,
                    }
                }
                browse.stop();
            });
    }

    /// Stop browsing; the stream closes within [`LISTENING`].
    #[frb(sync)]
    pub fn stop(&self) {
        self.browse.stop();
    }
}
