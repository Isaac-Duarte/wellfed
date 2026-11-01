//! `crate::ble` where BlueZ does not exist. The commands stay in the CLI so
//! it builds and its other commands work, and both commands that need the
//! radio check [`AVAILABLE`] and refuse before they change anything. A feeder
//! can still be set up from the Android app, which brings its own Bluetooth.

use std::convert::Infallible;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Result, bail};
use wellfed_install::{Bluetooth, PlatformError, SetupLink};

const NO_BLUETOOTH: &str = "this build of wellfed has no Bluetooth: it reaches a feeder through \
    BlueZ, which only Linux has";

/// Whether this build can reach a feeder over Bluetooth.
pub(crate) const AVAILABLE: bool = false;

/// A feeder's Bluetooth address, parsed so that the command line means the
/// same on every host, and refused when the link would open.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Address;

impl FromStr for Address {
    type Err = Infallible;

    fn from_str(_: &str) -> Result<Self, Infallible> {
        Ok(Self)
    }
}

/// A link that cannot be opened, so its methods are unreachable by type.
pub(crate) struct Link(Infallible);

impl Link {
    pub(crate) async fn open(_: Option<Address>) -> Result<Self> {
        bail!(NO_BLUETOOTH)
    }
}

/// The computer's radio, which this build cannot open.
pub(crate) struct Bluez {
    pub(crate) address: Option<Address>,
}

impl Bluetooth for Bluez {
    type Link = Link;

    async fn open(&mut self) -> Result<Link, PlatformError> {
        Ok(Link::open(self.address).await?)
    }
}

impl SetupLink for Link {
    async fn send(&mut self, _: &[u8]) -> Result<(), PlatformError> {
        match self.0 {}
    }

    async fn receive(&mut self, _: Duration) -> Result<Vec<u8>, PlatformError> {
        match self.0 {}
    }

    async fn close(self) {
        match self.0 {}
    }
}
