//! Bluetooth setup's radio on the desktops: the adapter's power, the scan
//! for feeders in setup mode, and one connection's control characteristic,
//! through `wellfed_ble`, the CLI's. Android's radio is
//! `flutter_reactive_ble`, in Dart, and never calls these.
//!
//! This moves pieces and notifications only: `SetupDriver` frames what goes
//! and reads what comes back, the same on every platform.
//!
//! Each stream's failures travel as values or end it, never through
//! `add_error`. A handle runs until its `stop` (or `close`), since a Dart
//! subscription's cancel is never heard here.

use flutter_rust_bridge::frb;
use tokio::sync::watch;

use crate::api::error::BridgeError;
use crate::bluetooth as radio;
use crate::frb_generated::StreamSink;

/// Whether this computer's Bluetooth is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdapterPower {
    PoweredOn,
    PoweredOff,
}

/// A feeder advertising the setup service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeardFeeder {
    /// The stack's own name for it, which [`open_bluetooth_link`] takes.
    pub id: String,
    /// The name it advertises; empty when it gives none.
    pub name: String,
    /// Its signal strength when last heard, in dBm; -127 when unknown.
    pub rssi: i16,
}

/// What a scan says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScanEvent {
    Heard {
        feeder: HeardFeeder,
    },
    /// The scan could not start or stopped working; the stream ends.
    Failed {
        error: BridgeError,
    },
}

/// Whether Bluetooth is on, now and as it changes, until [`Self::stop`].
#[frb(opaque)]
pub struct PowerWatch {
    stop: watch::Sender<bool>,
}

impl PowerWatch {
    #[frb(sync)]
    pub fn new() -> Self {
        Self {
            stop: watch::channel(false).0,
        }
    }

    /// Report the power now, then each change. The stream ends with
    /// nothing said when the stack can't be opened: the scan says why.
    pub async fn run(&self, power: StreamSink<AdapterPower>) {
        let stopped = self.stop.subscribe();
        radio::power(stopped, |state| power.add(state).is_ok()).await;
    }

    #[frb(sync)]
    pub fn stop(&self) {
        self.stop.send_replace(true);
    }
}

/// One scan for feeders in setup mode, until [`Self::stop`].
#[frb(opaque)]
pub struct SetupScan {
    stop: watch::Sender<bool>,
}

impl SetupScan {
    #[frb(sync)]
    pub fn new() -> Self {
        Self {
            stop: watch::channel(false).0,
        }
    }

    /// Report each feeder heard, again on each round it is still heard.
    pub async fn run(&self, events: StreamSink<ScanEvent>) {
        let stopped = self.stop.subscribe();
        radio::scan(stopped, |event| events.add(event).is_ok()).await;
    }

    /// Stop the scan; its stream ends within a round.
    #[frb(sync)]
    pub fn stop(&self) {
        self.stop.send_replace(true);
    }
}

/// A connected feeder's control characteristic.
#[frb(opaque)]
pub struct BluetoothLink {
    link: radio::Link,
    closed: watch::Sender<bool>,
}

/// Connect to the feeder a scan heard as `id`, and turn its notifications
/// on.
pub async fn open_bluetooth_link(id: String) -> Result<BluetoothLink, BridgeError> {
    Ok(BluetoothLink {
        link: radio::Link::open(&id).await?,
        closed: watch::channel(false).0,
    })
}

impl BluetoothLink {
    /// Each notification's bytes, in order. The stream ends when the link
    /// drops or closes. Only the first call hears anything.
    pub async fn notifications(&self, notified: StreamSink<Vec<u8>>) {
        let closed = self.closed.subscribe();
        self.link
            .notifications(closed, |bytes| notified.add(bytes).is_ok())
            .await;
    }

    /// Write one piece with a write request, acknowledged before this
    /// returns.
    pub async fn write(&self, piece: Vec<u8>) -> Result<(), BridgeError> {
        self.link.write(&piece).await
    }

    /// Hang up; the notifications end.
    pub async fn close(&self) {
        self.closed.send_replace(true);
        self.link.close().await;
    }
}
