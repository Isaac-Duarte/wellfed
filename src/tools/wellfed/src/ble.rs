//! `wellfed setup`'s radio half: find a feeder in setup mode and carry
//! `wellfed_client::setup`'s messages over its control characteristic, as
//! `wellfed_install`'s [`SetupLink`]. The radio and the framing are
//! `wellfed_ble`'s, which the desktop app shares.
//!
//! macOS asks for Bluetooth access on behalf of the app that started this
//! process, usually the terminal, under Privacy & Security in System
//! Settings.

use std::time::Duration;

use anyhow::Result;
use wellfed_ble::{Connection, Framed, Radio};
use wellfed_install::{PlatformError, SetupLink};

pub(crate) use wellfed_ble::Address;

/// One connected feeder's control characteristic, framed.
pub(crate) struct Link {
    framed: Framed<Connection>,
}

impl Link {
    /// Find the feeder (`address`, or the first advertising setup), connect
    /// and subscribe.
    pub(crate) async fn open(address: Option<Address>) -> Result<Self> {
        let radio = Radio::open().await?;
        let feeder = radio.find(address).await?;
        eprintln!("Found a feeder at {}; connecting…", feeder.address());
        let (connection, notifications) = radio.connect(&feeder).await?;
        Ok(Self {
            framed: Framed::new(connection, notifications),
        })
    }
}

impl SetupLink for Link {
    /// Send one message, framed. Each piece is acknowledged before the next.
    async fn send(&mut self, message: &[u8]) -> Result<(), PlatformError> {
        Ok(self.framed.send(message).await?)
    }

    /// The next message, waiting at most `wait`.
    async fn receive(&mut self, wait: Duration) -> Result<Vec<u8>, PlatformError> {
        Ok(self.framed.receive(wait).await?)
    }

    /// Hang up, bounded: a stack can stall disconnecting a device the feeder
    /// already dropped.
    async fn close(self) {
        self.framed.into_control().disconnect().await;
    }
}
