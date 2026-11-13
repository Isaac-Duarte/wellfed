//! The adapter, the scan and one connection, through `btleplug`.
//!
//! Each stack's habits, as met by the CLI's earlier code, shape this:
//!
//! - A filtered discovery once crashed BlueZ's bluetoothd (5.87), so on
//!   Linux the scan names no service and the service is checked here.
//!   `btleplug` still sets a discovery filter there, with no service in it.
//! - Every stack lists devices it heard before, so only one heard lately
//!   counts as advertising now ([`Scan::heard`]). A feeder still linked
//!   from a run before stops advertising; when the caller named it, being
//!   connected counts.
//! - CoreBluetooth and WinRT forget a device when its link closes, so a
//!   feeder named by its id is listened for again ([`Radio::heard_as`]).
//! - The adapter has one discovery, shared by every scan ([`Scans`]).
//! - BlueZ connects only to a device it has heard since it last forgot it,
//!   so a connection always follows a scan, even to a named feeder.
//! - A notification sent as soon as notifications are enabled must have
//!   somewhere to go: the stream is taken before subscribing.
//! - CoreBluetooth knows a peripheral only through the central that heard
//!   it, so the connection goes through the same [`Radio`] that scanned.

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use btleplug::api::{
    Central, CentralEvent, CentralState, Characteristic, Manager as _, Peripheral as _,
    PeripheralProperties, ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures_util::{Stream, StreamExt, stream};
use tokio::runtime::Handle;
use tokio::time::timeout;
use uuid::Uuid;
use wellfed_client::setup::{CONTROL, SERVICE};

use crate::shared::{Discovery, Links, Scans, Sightings, look_again};
use crate::{
    Address, CLOSE_TIMEOUT, CONNECT_TIMEOUT, Control, Error, FIND_TIMEOUT, Notifications,
    RESCAN_WAIT,
};

/// How often a scan reads what the adapter has heard.
pub(crate) const HEARING: Duration = Duration::from_millis(500);

/// Whether the adapter is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Power {
    On,
    Off,
}

impl Power {
    /// `None` for a state the stack does not know yet, or one turning the
    /// adapter on can't fix (no permission, no radio): the scan says what
    /// is wrong then.
    fn of(state: &CentralState) -> Option<Self> {
        match state {
            CentralState::PoweredOn => Some(Self::On),
            CentralState::PoweredOff => Some(Self::Off),
            CentralState::Unknown => None,
        }
    }
}

/// This computer's first Bluetooth adapter, shared by every scan and
/// connection made through it.
#[derive(Clone)]
pub struct Radio {
    adapter: Adapter,
    sightings: Arc<StdMutex<Sightings>>,
    scans: Arc<Scans<AdapterDiscovery>>,
    links: Arc<StdMutex<Links>>,
}

impl Radio {
    /// Open the stack and take its first adapter. On macOS this is where
    /// the system first asks whether the app may use Bluetooth.
    pub async fn open() -> Result<Self, Error> {
        let manager = Manager::new().await.map_err(Error::Open)?;
        let adapter = manager
            .adapters()
            .await
            .map_err(Error::Open)?
            .into_iter()
            .next()
            .ok_or(Error::NoAdapter)?;

        let sightings = Arc::new(StdMutex::new(Sightings::default()));
        let scans = Arc::new(Scans::new(AdapterDiscovery {
            adapter: adapter.clone(),
        }));
        let events = adapter.events().await.map_err(Error::State)?;
        tokio::spawn(follow_adapter(
            events,
            Arc::downgrade(&sightings),
            Arc::downgrade(&scans),
        ));
        Ok(Self {
            adapter,
            sightings,
            scans,
            links: Arc::new(StdMutex::new(Links::default())),
        })
    }

    /// Whether the adapter is on now; `None` while the stack can't say.
    pub async fn power_now(&self) -> Result<Option<Power>, Error> {
        let state = self.adapter.adapter_state().await.map_err(Error::State)?;
        Ok(Power::of(&state))
    }

    /// Whether the adapter is on, now and each time it changes. A state
    /// the stack doesn't know is left out.
    pub async fn power(&self) -> Result<impl Stream<Item = Power> + Send + 'static, Error> {
        // The changes first, so one between the two reads is not lost.
        let events = self.adapter.events().await.map_err(Error::State)?;
        let now = self.power_now().await?;
        let changes = events.filter_map(|event| async move {
            match event {
                CentralEvent::StateUpdate(state) => Power::of(&state),
                _ => None,
            }
        });
        Ok(stream::iter(now).chain(changes))
    }

    /// Start listening for feeders in setup mode, or `address` alone.
    /// Discovery is shared: it runs while any scan does.
    pub async fn scan(&self, address: Option<Address>) -> Result<Scan, Error> {
        if self.power_now().await? == Some(Power::Off) {
            return Err(Error::Off);
        }
        self.scans.join().await?;
        Ok(Scan {
            radio: self.clone(),
            address,
            left: false,
        })
    }

    /// The feeder advertising setup: `address` if given, otherwise the
    /// first one heard within [`FIND_TIMEOUT`]. The scan stops either way.
    pub async fn find(&self, address: Option<Address>) -> Result<Heard, Error> {
        let scan = self.scan(address).await?;
        let found = timeout(FIND_TIMEOUT, async {
            loop {
                if let Some(heard) = scan.heard().await?.into_iter().next() {
                    return Ok(heard);
                }
                tokio::time::sleep(HEARING).await;
            }
        })
        .await;
        scan.stop().await;
        found.map_err(|_| Error::NotFound)?
    }

    /// The feeder a scan reported as `id` ([`Heard::id`]). macOS and Windows
    /// forget a device when its link closes, so a feeder the adapter no
    /// longer knows is listened for again, for at most [`RESCAN_WAIT`].
    pub async fn heard_as(&self, id: &str) -> Result<Heard, Error> {
        let (found, scan) = look_again(
            || self.known_as(id),
            || self.scan(None),
            HEARING,
            RESCAN_WAIT,
        )
        .await;
        if let Some(scan) = scan {
            scan.stop().await;
        }
        found?.ok_or_else(|| Error::Gone(id.to_owned()))
    }

    async fn known_as(&self, id: &str) -> Result<Option<Heard>, Error> {
        for peripheral in self.adapter.peripherals().await.map_err(Error::Scan)? {
            if peripheral.id().to_string() == id {
                let properties = peripheral.properties().await.map_err(Error::Scan)?;
                return Ok(Some(Heard::new(peripheral, properties.as_ref())));
            }
        }
        Ok(None)
    }

    /// Connect to the feeder a scan heard as `id`, as [`Self::connect`].
    /// Every step is bounded, so this ends within [`OPEN_LIMIT`].
    pub async fn open_heard(&self, id: &str) -> Result<(Connection, Notifications), Error> {
        let feeder = self.heard_as(id).await?;
        self.connect(&feeder).await
    }

    /// Connect to `feeder`, find its control characteristic and turn its
    /// notifications on. A connection that fails partway, or whose caller
    /// stops waiting for it, is hung up, unless a newer one to the same
    /// feeder has begun: the stack has one link per device, which both share.
    pub async fn connect(&self, feeder: &Heard) -> Result<(Connection, Notifications), Error> {
        let peripheral = feeder.peripheral.clone();
        let hang_up = HangUp::claim(&self.links, &peripheral);
        // Before connecting, so a drop during the setup is not missed.
        let events = self.adapter.events().await.map_err(Error::State)?;
        match open(&peripheral, events).await {
            Ok((control, notifications)) => {
                let connection = Connection {
                    peripheral,
                    control,
                    hang_up,
                };
                Ok((connection, notifications))
            }
            Err(error) => {
                hang_up.now().await;
                Err(error)
            }
        }
    }
}

/// Follow the adapter until the radio goes: note when each device is heard
/// advertising, and start discovery again when the adapter comes back on,
/// since turning it off ends discovery on every stack while scans still
/// hold it.
async fn follow_adapter(
    mut events: Pin<Box<dyn Stream<Item = CentralEvent> + Send>>,
    sightings: Weak<StdMutex<Sightings>>,
    scans: Weak<Scans<AdapterDiscovery>>,
) {
    while let Some(event) = events.next().await {
        if let CentralEvent::StateUpdate(CentralState::PoweredOn) = event {
            let Some(scans) = scans.upgrade() else { return };
            scans.resume().await;
            continue;
        }
        // Read on macOS and Windows alone (`Scan::advertising`).
        if cfg!(target_os = "linux") {
            continue;
        }
        let id = match event {
            CentralEvent::DeviceDiscovered(id)
            | CentralEvent::DeviceUpdated(id)
            | CentralEvent::ManufacturerDataAdvertisement { id, .. }
            | CentralEvent::ServiceDataAdvertisement { id, .. }
            | CentralEvent::ServicesAdvertisement { id, .. }
            | CentralEvent::RssiUpdate { id, .. } => id,
            _ => continue,
        };
        let Some(sightings) = sightings.upgrade() else {
            return;
        };
        sightings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .heard(id.to_string(), Instant::now());
    }
}

/// The adapter's own discovery, which [`Scans`] shares out.
struct AdapterDiscovery {
    adapter: Adapter,
}

impl Discovery for AdapterDiscovery {
    /// Bounded, since every other scan waits on it.
    async fn start(&self) -> Result<(), Error> {
        let services = if cfg!(target_os = "linux") {
            Vec::new()
        } else {
            vec![Uuid::from_u128(SERVICE)]
        };
        timeout(
            CONNECT_TIMEOUT,
            self.adapter.start_scan(ScanFilter { services }),
        )
        .await
        .map_err(|_| Error::ScanTimeout)?
        .map_err(Error::Scan)
    }

    async fn stop(&self) {
        // A stack that fails to stop, or doesn't say, has nothing more to say.
        let _ = timeout(CONNECT_TIMEOUT, self.adapter.stop_scan()).await;
    }
}

async fn open(
    peripheral: &Peripheral,
    events: Pin<Box<dyn Stream<Item = CentralEvent> + Send>>,
) -> Result<(Characteristic, Notifications), Error> {
    if !peripheral.is_connected().await.map_err(Error::Connect)? {
        timeout(CONNECT_TIMEOUT, peripheral.connect())
            .await
            .map_err(|_| Error::ConnectTimeout)?
            .map_err(Error::Connect)?;
    }
    timeout(CONNECT_TIMEOUT, peripheral.discover_services())
        .await
        .map_err(|_| Error::ServicesTimeout)?
        .map_err(Error::Connect)?;
    let control = control(peripheral)?;

    let stream = peripheral.notifications().await.map_err(Error::Connect)?;
    timeout(CONNECT_TIMEOUT, peripheral.subscribe(&control))
        .await
        .map_err(|_| Error::SubscribeTimeout)?
        .map_err(Error::Connect)?;

    // Neither BlueZ nor CoreBluetooth ends the notifications when the
    // feeder drops the link, so its disconnection ends them here.
    let id = peripheral.id();
    let dropped = events
        .filter(move |event| {
            std::future::ready(
                matches!(event, CentralEvent::DeviceDisconnected(gone) if *gone == id),
            )
        })
        .into_future();
    let wanted = control.uuid;
    let notifications = stream
        .filter_map(move |notification| async move {
            (notification.uuid == wanted).then_some(notification.value)
        })
        .take_until(dropped);
    Ok((control, Box::pin(notifications)))
}

fn control(peripheral: &Peripheral) -> Result<Characteristic, Error> {
    let (service, control) = (Uuid::from_u128(SERVICE), Uuid::from_u128(CONTROL));
    peripheral
        .characteristics()
        .into_iter()
        .find(|characteristic| {
            characteristic.service_uuid == service && characteristic.uuid == control
        })
        .ok_or(Error::NoSetupService)
}

/// One scan on the adapter, until [`Self::stop`]. Dropped unstopped, it
/// lets go of discovery on the runtime it is dropped on, if any.
pub struct Scan {
    radio: Radio,
    address: Option<Address>,
    left: bool,
}

impl Scan {
    /// The feeders advertising setup now, as far as the adapter has heard.
    /// A feeder shows up again on each read while it is heard.
    pub async fn heard(&self) -> Result<Vec<Heard>, Error> {
        let service = Uuid::from_u128(SERVICE);
        let mut heard = Vec::new();
        let peripherals = self
            .radio
            .adapter
            .peripherals()
            .await
            .map_err(Error::Scan)?;
        for peripheral in peripherals {
            if self
                .address
                .is_some_and(|address| !address.names(&peripheral))
            {
                continue;
            }
            let Some(properties) = peripheral.properties().await.map_err(Error::Scan)? else {
                continue;
            };
            if !properties.services.contains(&service) {
                continue;
            }
            let advertising = self.advertising(&peripheral, &properties)
                || (self.address.is_some()
                    && peripheral.is_connected().await.map_err(Error::Scan)?);
            if advertising {
                heard.push(Heard::new(peripheral, Some(&properties)));
            }
        }
        Ok(heard)
    }

    /// Whether `peripheral` is advertising now. BlueZ drops a device's
    /// signal strength when discovery stops, and reports a change only past
    /// 8 dBm, so a feeder lying still may go quiet there for long: a signal
    /// strength is the test on Linux. CoreBluetooth and WinRT keep the last
    /// one for good, but report every advertisement, so elsewhere it is a
    /// sighting in the last [`crate::shared::RECENT`].
    fn advertising(&self, peripheral: &Peripheral, properties: &PeripheralProperties) -> bool {
        if cfg!(target_os = "linux") {
            return properties.rssi.is_some();
        }
        self.radio
            .sightings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .recent(&peripheral.id().to_string(), Instant::now())
    }

    /// Stop this scan; discovery stops with the last one.
    pub async fn stop(mut self) {
        self.left = true;
        self.radio.scans.leave().await;
    }
}

impl Drop for Scan {
    fn drop(&mut self) {
        if self.left {
            return;
        }
        if let Ok(runtime) = Handle::try_current() {
            let scans = self.radio.scans.clone();
            runtime.spawn(async move { scans.leave().await });
        }
    }
}

/// A feeder in setup mode, as a scan heard it.
#[derive(Clone)]
pub struct Heard {
    peripheral: Peripheral,
    id: String,
    address: String,
    name: Option<String>,
    rssi: Option<i16>,
}

impl Heard {
    fn new(
        peripheral: Peripheral,
        properties: Option<&btleplug::api::PeripheralProperties>,
    ) -> Self {
        Self {
            id: peripheral.id().to_string(),
            // CoreBluetooth hides the address and names a device by its id.
            address: if cfg!(target_os = "macos") {
                peripheral.id().to_string()
            } else {
                peripheral.address().to_string()
            },
            name: properties.and_then(|properties| properties.local_name.clone()),
            rssi: properties.and_then(|properties| properties.rssi),
            peripheral,
        }
    }

    /// The stack's own name for it, which [`Radio::heard_as`] and
    /// [`Radio::open_heard`] take back:
    /// a UUID on macOS, the address elsewhere (on Linux, BlueZ's path).
    pub fn id(&self) -> &str {
        &self.id
    }

    /// It as an [`crate::Address`] is written: the address, or on macOS the
    /// id.
    pub fn address(&self) -> &str {
        &self.address
    }

    /// The name it advertises, if any.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Its signal strength when last heard, in dBm.
    pub const fn rssi(&self) -> Option<i16> {
        self.rssi
    }
}

/// One connected feeder's control characteristic. Dropped without
/// [`Self::disconnect`], it hangs up on the runtime it was opened on, so a
/// link nobody holds never keeps the feeder's only connection.
pub struct Connection {
    peripheral: Peripheral,
    control: Characteristic,
    hang_up: HangUp,
}

impl Connection {
    /// Hang up, waiting at most [`CLOSE_TIMEOUT`], unless a newer
    /// connection to the same feeder has begun since.
    pub async fn disconnect(&self) {
        self.hang_up.now().await;
    }
}

/// One connection's hang-up, which happens once: when asked, or when it is
/// dropped unasked (a connection whose caller stopped waiting, or that
/// nobody holds), on the runtime it began on. Only the newest connection
/// to a feeder hangs up: the stacks keep one link per device, so an older
/// one hanging up late would cut the newer one.
struct HangUp {
    peripheral: Peripheral,
    links: Arc<StdMutex<Links>>,
    id: String,
    claim: u64,
    runtime: Option<Handle>,
    armed: AtomicBool,
}

impl HangUp {
    fn claim(links: &Arc<StdMutex<Links>>, peripheral: &Peripheral) -> Self {
        let id = peripheral.id().to_string();
        let claim = links
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .claim(&id);
        Self {
            peripheral: peripheral.clone(),
            links: links.clone(),
            id,
            claim,
            runtime: Handle::try_current().ok(),
            armed: AtomicBool::new(true),
        }
    }

    /// Whether this is the one to hang up: armed, and still the newest.
    fn take(&self) -> bool {
        self.armed.swap(false, Ordering::SeqCst)
            && self
                .links
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .release(&self.id, self.claim)
    }

    async fn now(&self) {
        if self.take() {
            let _ = timeout(CLOSE_TIMEOUT, self.peripheral.disconnect()).await;
        }
    }
}

impl Drop for HangUp {
    fn drop(&mut self) {
        let Some(runtime) = self.runtime.clone() else {
            return;
        };
        if self.take() {
            let peripheral = self.peripheral.clone();
            runtime.spawn(async move {
                let _ = timeout(CLOSE_TIMEOUT, peripheral.disconnect()).await;
            });
        }
    }
}

impl Control for Connection {
    /// A write request, so the piece is acknowledged and arrives in order.
    async fn write(&self, piece: &[u8]) -> Result<(), Error> {
        timeout(
            CONNECT_TIMEOUT,
            self.peripheral
                .write(&self.control, piece, WriteType::WithResponse),
        )
        .await
        .map_err(|_| Error::WriteTimeout)?
        .map_err(Error::Write)
    }
}
