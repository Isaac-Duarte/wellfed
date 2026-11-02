//! `wellfed setup`'s radio half: find a feeder in setup mode and carry
//! `wellfed_client::setup`'s messages over its control characteristic, as
//! `wellfed_install`'s [`SetupLink`].
//!
//! The framing is here, and each host's Bluetooth stack below it: [`bluez`]
//! on Linux, [`corebluetooth`] on macOS. The Android app has its own, in Dart.

use std::pin::Pin;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::{Stream, StreamExt};
use tokio::time::timeout;
use wellfed_client::setup::{Deframer, frame};
use wellfed_install::{PlatformError, SetupLink};

#[cfg(target_os = "linux")]
mod bluez;
#[cfg(target_os = "linux")]
use bluez as platform;
#[cfg(target_os = "macos")]
mod corebluetooth;
#[cfg(target_os = "macos")]
use corebluetooth as platform;

pub(crate) use platform::Address;

const FIND_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);
/// Control writes fit the smallest ATT payload, so they never depend on the
/// MTU exchange having happened.
const CONTROL_CHUNK: usize = 20;
const NOT_FOUND: &str = "no feeder in setup mode nearby: hold Lock then Mic for 5 s on an unlocked feeder on mains power";

/// What the control characteristic sends, piece by piece.
type Notifications = Pin<Box<dyn Stream<Item = Vec<u8>> + Send>>;

/// One connected feeder's control characteristic, framed.
pub(crate) struct Link {
    connection: platform::Connection,
    notifications: Notifications,
    deframer: Deframer,
}

impl Link {
    /// Find the feeder (`address`, or the first advertising setup), connect
    /// and subscribe.
    pub(crate) async fn open(address: Option<Address>) -> Result<Self> {
        let (connection, notifications) = platform::Connection::open(address).await?;
        Ok(Self {
            connection,
            notifications,
            deframer: Deframer::default(),
        })
    }

    /// Send one message, framed. Each piece is acknowledged before the next.
    pub(crate) async fn send(&mut self, message: &[u8]) -> Result<()> {
        for piece in frame(message)?.chunks(CONTROL_CHUNK) {
            timeout(CONNECT_TIMEOUT, self.connection.write(piece))
                .await
                .context("writing to the feeder timed out")?
                .context("write to the feeder")?;
        }
        Ok(())
    }

    /// The next message, waiting at most `wait`.
    pub(crate) async fn receive(&mut self, wait: Duration) -> Result<Vec<u8>> {
        timeout(wait, async {
            loop {
                if let Some(message) = self.deframer.next_message()? {
                    return Ok(message);
                }
                let notification = self
                    .notifications
                    .next()
                    .await
                    .context("the feeder dropped the link")?;
                self.deframer.push(&notification);
            }
        })
        .await
        .context("no answer from the feeder in time")?
    }

    /// Hang up, bounded: a stack can stall disconnecting a device the feeder
    /// already dropped.
    pub(crate) async fn close(self) {
        self.connection.disconnect(CLOSE_TIMEOUT).await;
    }
}

impl SetupLink for Link {
    async fn send(&mut self, message: &[u8]) -> Result<(), PlatformError> {
        Ok(Self::send(self, message).await?)
    }

    async fn receive(&mut self, wait: Duration) -> Result<Vec<u8>, PlatformError> {
        Ok(Self::receive(self, wait).await?)
    }

    async fn close(self) {
        Self::close(self).await;
    }
}
