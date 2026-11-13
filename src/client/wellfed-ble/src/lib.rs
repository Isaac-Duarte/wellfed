//! The feeder's Bluetooth setup service on a computer, through `btleplug`:
//! CoreBluetooth on macOS, BlueZ on Linux and WinRT on Windows. The CLI's
//! `wellfed setup` and the desktop app both carry setup over it; the
//! Android app has its own radio, in Dart.
//!
//! This crate moves bytes and knows no protocol. [`Radio`] reports the
//! adapter's power, scans for feeders advertising the setup service and
//! connects to one; a [`Connection`] writes pieces to the control
//! characteristic and hands back its notifications. [`Framed`] puts whole
//! messages over any [`Control`], with `wellfed_client::setup`'s framing,
//! for a caller that runs the session in Rust (the CLI); the app frames in
//! `wellfed_client` itself and moves pieces only.
//!
//! None of this has run against a feeder on Linux or Windows through
//! `btleplug`, and on macOS only through the CLI's earlier code it was
//! moved from.

#![forbid(unsafe_code)]

mod address;
mod framed;
mod radio;

use std::pin::Pin;
use std::time::Duration;

use futures_util::Stream;

pub use address::{Address, AddressError};
pub use framed::{Control, Framed};
pub use radio::{Connection, Heard, Power, Radio, Scan};

/// How long [`Radio::find`] listens for a feeder in setup mode.
pub const FIND_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a feeder gets to answer a connection, its services' discovery,
/// the subscription and each write.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a hang-up may take: a stack can stall disconnecting a device the
/// feeder already dropped.
pub const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);
/// Control writes fit the smallest ATT payload, so they never depend on the
/// MTU exchange having happened.
pub const PIECE: usize = 20;

/// What the control characteristic sends, notification by notification.
/// It ends when the link drops.
pub type Notifications = Pin<Box<dyn Stream<Item = Vec<u8>> + Send>>;

/// What went wrong on the radio. Every `btleplug` failure is kept whole.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "this computer's Bluetooth could not be opened: check it is on, and that this app may use it"
    )]
    Open(#[source] btleplug::Error),
    #[error("this computer has no Bluetooth adapter")]
    NoAdapter,
    #[error("this computer's Bluetooth is off")]
    Off,
    #[error("the scan for feeders failed")]
    Scan(#[source] btleplug::Error),
    #[error(
        "no feeder in setup mode nearby: hold Lock then Mic for 5 s on an unlocked feeder on mains power"
    )]
    NotFound,
    #[error("the feeder {0} is no longer heard; look for it again")]
    Gone(String),
    #[error("connecting to the feeder timed out")]
    ConnectTimeout,
    #[error("the feeder's services did not resolve in time")]
    ServicesTimeout,
    #[error("enabling the feeder's notifications timed out")]
    SubscribeTimeout,
    #[error("connecting to the feeder failed")]
    Connect(#[source] btleplug::Error),
    #[error("the device has no setup service")]
    NoSetupService,
    #[error("writing to the feeder timed out")]
    WriteTimeout,
    #[error("writing to the feeder failed")]
    Write(#[source] btleplug::Error),
    #[error("the feeder dropped the link")]
    Dropped,
    #[error("no answer from the feeder in time")]
    Silent,
    #[error(transparent)]
    Frame(#[from] wellfed_client::setup::Error),
}
