//! The radio behind [`crate::api::bluetooth`]. On the desktops it is
//! `wellfed_ble`'s, the CLI's; Android has its radio in Dart, so there each
//! call fails or ends at once if it were ever made.

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub(crate) use desktop::{Link, power, scan};
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use elsewhere::{Link, power, scan};

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
mod desktop {
    use std::future::Future;
    use std::sync::Mutex;
    use std::time::Duration;

    use futures_util::{Stream, StreamExt};
    use tokio::sync::{OnceCell, watch};
    use tokio::time::timeout;
    use wellfed_ble::{Connection, Control, Notifications, Radio, Scan};

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

    /// The radio, opened within [`wellfed_ble::CONNECT_TIMEOUT`]: on macOS
    /// the system may still be asking whether the app may use Bluetooth.
    async fn radio() -> Result<&'static Radio, wellfed_ble::Error> {
        RADIO
            .get_or_try_init(|| async {
                timeout(wellfed_ble::CONNECT_TIMEOUT, Radio::open())
                    .await
                    .map_err(|_| wellfed_ble::Error::OpenTimeout)?
            })
            .await
    }

    impl From<wellfed_ble::Error> for BridgeError {
        fn from(error: wellfed_ble::Error) -> Self {
            Self::new(ErrorKind::Bluetooth, error)
        }
    }

    pub(crate) async fn power(
        stopped: watch::Receiver<bool>,
        tell: impl FnMut(AdapterPower) -> bool,
    ) {
        let Ok(radio) = radio().await else { return };
        let Ok(changes) = radio.power().await else {
            return;
        };
        follow(changes, stopped, tell).await;
    }

    /// Tell each power `changes` reports until it ends, `stopped` is set or
    /// nobody listens.
    async fn follow(
        changes: impl Stream<Item = wellfed_ble::Power>,
        mut stopped: watch::Receiver<bool>,
        mut tell: impl FnMut(AdapterPower) -> bool,
    ) {
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

    /// What the scan's loop needs of a running scan.
    trait Hearing {
        /// The feeders heard advertising now.
        fn heard(&self) -> impl Future<Output = Result<Vec<HeardFeeder>, BridgeError>> + Send;
        fn stop(self) -> impl Future<Output = ()> + Send;
    }

    impl Hearing for Scan {
        async fn heard(&self) -> Result<Vec<HeardFeeder>, BridgeError> {
            let heard = Scan::heard(self).await?;
            Ok(heard
                .iter()
                .map(|feeder| HeardFeeder {
                    id: feeder.id().to_owned(),
                    name: feeder.name().unwrap_or_default().to_owned(),
                    rssi: feeder.rssi().unwrap_or(RSSI_UNKNOWN),
                })
                .collect())
        }

        async fn stop(self) {
            Scan::stop(self).await;
        }
    }

    pub(crate) async fn scan(stopped: watch::Receiver<bool>, tell: impl FnMut(ScanEvent) -> bool) {
        let started = match radio().await {
            Ok(radio) => radio.scan(None).await.map_err(BridgeError::from),
            Err(error) => Err(error.into()),
        };
        hear(started, stopped, tell).await;
    }

    /// Tell each feeder `scan` hears, every [`HEARING`], until `stopped` is
    /// set or nobody listens, then stop it. A scan that fails is told once,
    /// and ends.
    async fn hear(
        scan: Result<impl Hearing, BridgeError>,
        mut stopped: watch::Receiver<bool>,
        mut tell: impl FnMut(ScanEvent) -> bool,
    ) {
        let scan = match scan {
            Ok(scan) => scan,
            Err(error) => {
                tell(ScanEvent::Failed { error });
                return;
            }
        };

        loop {
            let heard = match scan.heard().await {
                Ok(heard) => heard,
                Err(error) => {
                    tell(ScanEvent::Failed { error });
                    break;
                }
            };
            let listening = heard
                .into_iter()
                .all(|feeder| tell(ScanEvent::Heard { feeder }));
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
            // Each step is bounded, so this ends within the radio's open and
            // `wellfed_ble::OPEN_LIMIT`, before Dart stops waiting.
            let radio = radio().await?;
            let (connection, notifications) = radio.open_heard(id).await?;
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

    #[cfg(test)]
    mod tests {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        use futures_util::stream;

        use super::*;

        fn feeder(id: &str) -> HeardFeeder {
            HeardFeeder {
                id: id.to_owned(),
                name: String::new(),
                rssi: -60,
            }
        }

        /// A scan that hears `rounds` in turn (the last again after that),
        /// or fails on the round after them when `failing`.
        struct FakeScan {
            rounds: Vec<Vec<HeardFeeder>>,
            reads: AtomicUsize,
            failing: bool,
            stops: Arc<AtomicUsize>,
        }

        impl FakeScan {
            fn new(rounds: Vec<Vec<HeardFeeder>>, stops: &Arc<AtomicUsize>) -> Self {
                Self {
                    rounds,
                    reads: AtomicUsize::new(0),
                    failing: false,
                    stops: stops.clone(),
                }
            }
        }

        impl Hearing for FakeScan {
            async fn heard(&self) -> Result<Vec<HeardFeeder>, BridgeError> {
                let read = self.reads.fetch_add(1, Ordering::SeqCst);
                if self.failing && read >= self.rounds.len() {
                    return Err(BridgeError::new(ErrorKind::Bluetooth, "the scan failed"));
                }
                let last = self.rounds.len().saturating_sub(1);
                Ok(self.rounds.get(read.min(last)).cloned().unwrap_or_default())
            }

            async fn stop(self) {
                self.stops.fetch_add(1, Ordering::SeqCst);
            }
        }

        fn heard(events: &[ScanEvent]) -> Vec<String> {
            events
                .iter()
                .filter_map(|event| match event {
                    ScanEvent::Heard { feeder } => Some(feeder.id.clone()),
                    ScanEvent::Failed { .. } => None,
                })
                .collect()
        }

        #[tokio::test(start_paused = true)]
        async fn a_scan_tells_each_round_until_stopped_and_then_stops() {
            let stops = Arc::new(AtomicUsize::new(0));
            let scan = FakeScan::new(vec![vec![], vec![feeder("a"), feeder("b")]], &stops);
            let (stop, stopped) = watch::channel(false);
            let mut events = Vec::new();

            let hearing = hear(Ok(scan), stopped, |event| {
                events.push(event);
                true
            });
            let stopping = async {
                tokio::time::sleep(HEARING * 2 + HEARING / 2).await;
                stop.send_replace(true);
            };
            tokio::join!(hearing, stopping);

            // Rounds at 0, 500 and 1000 ms: nothing, then both twice.
            assert_eq!(heard(&events), ["a", "b", "a", "b"]);
            assert_eq!(stops.load(Ordering::SeqCst), 1);
        }

        #[tokio::test(start_paused = true)]
        async fn a_scan_nobody_listens_to_stops() {
            let stops = Arc::new(AtomicUsize::new(0));
            let scan = FakeScan::new(vec![vec![feeder("a")]], &stops);
            let (_stop, stopped) = watch::channel(false);
            let mut told = 0;

            hear(Ok(scan), stopped, |_| {
                told += 1;
                false
            })
            .await;

            assert_eq!(told, 1);
            assert_eq!(stops.load(Ordering::SeqCst), 1);
        }

        #[tokio::test(start_paused = true)]
        async fn a_scan_that_fails_says_so_once_and_stops() {
            let stops = Arc::new(AtomicUsize::new(0));
            let scan = FakeScan {
                failing: true,
                ..FakeScan::new(vec![vec![feeder("a")]], &stops)
            };
            let (_stop, stopped) = watch::channel(false);
            let mut events = Vec::new();

            hear(Ok(scan), stopped, |event| {
                events.push(event);
                true
            })
            .await;

            assert_eq!(heard(&events), ["a"]);
            assert!(matches!(events.last(), Some(ScanEvent::Failed { .. })));
            assert_eq!(stops.load(Ordering::SeqCst), 1);
        }

        #[tokio::test]
        async fn a_scan_that_could_not_start_says_why() {
            let (_stop, stopped) = watch::channel(false);
            let mut events = Vec::new();
            let refused: Result<FakeScan, BridgeError> = Err(wellfed_ble::Error::Off.into());

            hear(refused, stopped, |event| {
                events.push(event);
                true
            })
            .await;

            assert!(matches!(
                events.as_slice(),
                [ScanEvent::Failed { error }] if error.kind == ErrorKind::Bluetooth
            ));
        }

        #[test]
        fn an_open_ends_before_dart_stops_waiting() {
            // `DesktopBluetooth.openWait` in Dart is 90 s, and its comment
            // says 75 s: the radio's open, then `open_heard`.
            let bounded = wellfed_ble::CONNECT_TIMEOUT + wellfed_ble::OPEN_LIMIT;
            assert_eq!(bounded, Duration::from_secs(75));
        }

        #[tokio::test]
        async fn power_is_told_as_it_changes_until_the_radio_stops_saying() {
            let changes = stream::iter([wellfed_ble::Power::Off, wellfed_ble::Power::On]);
            let (_stop, stopped) = watch::channel(false);
            let mut told = Vec::new();

            follow(changes, stopped, |power| {
                told.push(power);
                true
            })
            .await;

            assert_eq!(told, [AdapterPower::PoweredOff, AdapterPower::PoweredOn]);
        }

        #[tokio::test]
        async fn a_stopped_power_watch_ends_with_nothing_more_told() {
            let (stop, stopped) = watch::channel(false);
            stop.send_replace(true);
            let mut told = Vec::new();

            follow(stream::pending(), stopped, |power| {
                told.push(power);
                true
            })
            .await;

            assert!(told.is_empty());
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
