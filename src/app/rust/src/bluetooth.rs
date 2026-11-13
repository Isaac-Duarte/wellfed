//! The radio behind [`crate::api::bluetooth`]. On the desktops it is
//! `wellfed_ble`'s, the CLI's; Android has its radio in Dart, so there each
//! call fails or ends at once if it were ever made.

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub(crate) use desktop::{Link, power, scan};
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use elsewhere::{Link, power, scan};

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
mod desktop {
    use std::sync::Mutex;
    use std::time::Duration;

    use futures_util::StreamExt;
    use tokio::sync::{OnceCell, watch};
    use wellfed_ble::{Connection, Control, Notifications, Radio};

    use crate::api::bluetooth::{AdapterPower, HeardFeeder, ScanEvent};
    use crate::api::error::{BridgeError, ErrorKind};

    /// How often a scan reports what the adapter hears: a feeder shows up
    /// again on each round while it advertises, as Android's scan repeats it.
    const HEARING: Duration = Duration::from_millis(500);

    /// A heard feeder's signal strength when the stack gave none: the weakest
    /// it reports.
    const RSSI_UNKNOWN: i16 = -127;

    /// The one adapter, opened on first use and kept: CoreBluetooth
    /// connects only to a peripheral the central that heard it knows, so
    /// the scan and the connection share it. A failed open is tried again.
    static RADIO: OnceCell<Radio> = OnceCell::const_new();

    async fn radio() -> Result<&'static Radio, wellfed_ble::Error> {
        RADIO.get_or_try_init(Radio::open).await
    }

    impl From<wellfed_ble::Error> for BridgeError {
        fn from(error: wellfed_ble::Error) -> Self {
            Self::new(ErrorKind::Bluetooth, error)
        }
    }

    pub(crate) async fn power(
        mut stopped: watch::Receiver<bool>,
        mut tell: impl FnMut(AdapterPower) -> bool,
    ) {
        let Ok(radio) = radio().await else { return };
        let Ok(changes) = radio.power().await else {
            return;
        };
        // Boxed rather than `pin!`, whose expansion flutter_rust_bridge's parser
        // cannot read.
        let mut changes = Box::pin(changes);
        loop {
            let power = tokio::select! {
                _ = stopped.wait_for(|stopped| *stopped) => return,
                power = changes.next() => power,
            };
            let Some(power) = power else { return };
            let power = match power {
                wellfed_ble::Power::On => AdapterPower::PoweredOn,
                wellfed_ble::Power::Off => AdapterPower::PoweredOff,
            };
            if !tell(power) {
                return;
            }
        }
    }

    pub(crate) async fn scan(
        mut stopped: watch::Receiver<bool>,
        mut tell: impl FnMut(ScanEvent) -> bool,
    ) {
        let scan = match radio().await {
            Ok(radio) => radio.scan(None).await,
            Err(error) => Err(error),
        };
        let scan = match scan {
            Ok(scan) => scan,
            Err(error) => {
                tell(ScanEvent::Failed {
                    error: error.into(),
                });
                return;
            }
        };

        loop {
            let heard = match scan.heard().await {
                Ok(heard) => heard,
                Err(error) => {
                    tell(ScanEvent::Failed {
                        error: error.into(),
                    });
                    break;
                }
            };
            let listening = heard.iter().all(|feeder| {
                tell(ScanEvent::Heard {
                    feeder: HeardFeeder {
                        id: feeder.id().to_owned(),
                        name: feeder.name().unwrap_or_default().to_owned(),
                        rssi: feeder.rssi().unwrap_or(RSSI_UNKNOWN),
                    },
                })
            });
            if !listening {
                break;
            }
            tokio::select! {
                _ = stopped.wait_for(|stopped| *stopped) => break,
                () = tokio::time::sleep(HEARING) => {}
            }
        }
        scan.stop().await;
    }

    pub(crate) struct Link {
        connection: Connection,
        notifications: Mutex<Option<Notifications>>,
    }

    impl Link {
        pub(crate) async fn open(id: &str) -> Result<Self, BridgeError> {
            let radio = radio().await?;
            let feeder = radio.heard_as(id).await?;
            let (connection, notifications) = radio.connect(&feeder).await?;
            Ok(Self {
                connection,
                notifications: Mutex::new(Some(notifications)),
            })
        }

        pub(crate) async fn notifications(
            &self,
            mut closed: watch::Receiver<bool>,
            mut tell: impl FnMut(Vec<u8>) -> bool,
        ) {
            let taken = self
                .notifications
                .lock()
                .ok()
                .and_then(|mut notifications| notifications.take());
            let Some(mut notifications) = taken else {
                return;
            };
            loop {
                let bytes = tokio::select! {
                    _ = closed.wait_for(|closed| *closed) => return,
                    bytes = notifications.next() => bytes,
                };
                let Some(bytes) = bytes else { return };
                if !tell(bytes) {
                    return;
                }
            }
        }

        pub(crate) async fn write(&self, piece: &[u8]) -> Result<(), BridgeError> {
            Ok(self.connection.write(piece).await?)
        }

        pub(crate) async fn close(&self) {
            self.connection.disconnect().await;
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod elsewhere {
    use tokio::sync::watch;

    use crate::api::bluetooth::{AdapterPower, ScanEvent};
    use crate::api::error::{BridgeError, ErrorKind};

    fn unavailable() -> BridgeError {
        BridgeError::new(
            ErrorKind::Bluetooth,
            "this platform's Bluetooth is not reached through the bridge",
        )
    }

    pub(crate) async fn power(
        _stopped: watch::Receiver<bool>,
        _tell: impl FnMut(AdapterPower) -> bool,
    ) {
    }

    pub(crate) async fn scan(
        _stopped: watch::Receiver<bool>,
        mut tell: impl FnMut(ScanEvent) -> bool,
    ) {
        tell(ScanEvent::Failed {
            error: unavailable(),
        });
    }

    pub(crate) enum Link {}

    impl Link {
        pub(crate) async fn open(_id: &str) -> Result<Self, BridgeError> {
            Err(unavailable())
        }

        pub(crate) async fn notifications(
            &self,
            _closed: watch::Receiver<bool>,
            _tell: impl FnMut(Vec<u8>) -> bool,
        ) {
            match *self {}
        }

        pub(crate) async fn write(&self, _piece: &[u8]) -> Result<(), BridgeError> {
            match *self {}
        }

        pub(crate) async fn close(&self) {
            match *self {}
        }
    }
}
