//! The adapter, the scan and one connection, through `btleplug`.
//!
//! Each stack's habits, as met by the CLI's earlier code, shape this:
//!
//! - A filtered discovery once crashed BlueZ's bluetoothd (5.87), so on
//!   Linux the scan names no service and the service is checked here.
//!   `btleplug` still sets a discovery filter there, with no service in it.
//! - BlueZ keeps devices it heard before and lists them before they are
//!   heard again, so only a device with a signal strength counts as
//!   advertising now. A feeder still linked from a run before stops
//!   advertising; when the caller named it, being connected counts.
//! - BlueZ connects only to a device it has heard since it last forgot it,
//!   so a connection always follows a scan, even to a named feeder.
//! - A notification sent as soon as notifications are enabled must have
//!   somewhere to go: the stream is taken before subscribing.
//! - CoreBluetooth knows a peripheral only through the central that heard
//!   it, so the connection goes through the same [`Radio`] that scanned.

use std::pin::Pin;
use std::time::Duration;

use btleplug::api::{
    Central, CentralEvent, CentralState, Characteristic, Manager as _, Peripheral as _, ScanFilter,
    WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures_util::{Stream, StreamExt, stream};
use tokio::time::timeout;
use uuid::Uuid;
use wellfed_client::setup::{CONTROL, SERVICE};

use crate::{Address, CLOSE_TIMEOUT, CONNECT_TIMEOUT, Control, Error, FIND_TIMEOUT, Notifications};

/// How often a scan reads what the adapter has heard.
const HEARING: Duration = Duration::from_millis(500);

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

/// This computer's first Bluetooth adapter.
#[derive(Clone)]
pub struct Radio {
    adapter: Adapter,
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
        Ok(Self { adapter })
    }

    /// Whether the adapter is on now; `None` while the stack can't say.
    pub async fn power_now(&self) -> Result<Option<Power>, Error> {
        let state = self.adapter.adapter_state().await.map_err(Error::Open)?;
        Ok(Power::of(&state))
    }

    /// Whether the adapter is on, now and each time it changes. A state
    /// the stack doesn't know is left out.
    pub async fn power(&self) -> Result<impl Stream<Item = Power> + Send + 'static, Error> {
        // The changes first, so one between the two reads is not lost.
        let events = self.adapter.events().await.map_err(Error::Open)?;
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
    pub async fn scan(&self, address: Option<Address>) -> Result<Scan, Error> {
        if self.power_now().await? == Some(Power::Off) {
            return Err(Error::Off);
        }
        let services = if cfg!(target_os = "linux") {
            Vec::new()
        } else {
            vec![Uuid::from_u128(SERVICE)]
        };
        self.adapter
            .start_scan(ScanFilter { services })
            .await
            .map_err(Error::Scan)?;
        Ok(Scan {
            adapter: self.adapter.clone(),
            address,
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

    /// The feeder a scan reported as `id` ([`Heard::id`]), if the adapter
    /// still knows it.
    pub async fn heard_as(&self, id: &str) -> Result<Heard, Error> {
        for peripheral in self.adapter.peripherals().await.map_err(Error::Scan)? {
            if peripheral.id().to_string() == id {
                let properties = peripheral.properties().await.map_err(Error::Scan)?;
                return Ok(Heard::new(peripheral, properties.as_ref()));
            }
        }
        Err(Error::Gone(id.to_owned()))
    }

    /// Connect to `feeder`, find its control characteristic and turn its
    /// notifications on. A connection that fails partway is hung up.
    pub async fn connect(&self, feeder: &Heard) -> Result<(Connection, Notifications), Error> {
        let peripheral = feeder.peripheral.clone();
        // Before connecting, so a drop during the setup is not missed.
        let events = self.adapter.events().await.map_err(Error::Connect)?;
        let opened = open(&peripheral, events).await;
        if opened.is_err() {
            let _ = timeout(CLOSE_TIMEOUT, peripheral.disconnect()).await;
        }
        opened
    }
}

async fn open(
    peripheral: &Peripheral,
    events: Pin<Box<dyn Stream<Item = CentralEvent> + Send>>,
) -> Result<(Connection, Notifications), Error> {
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
    let connection = Connection {
        peripheral: peripheral.clone(),
        control,
    };
    Ok((connection, Box::pin(notifications)))
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

/// A scan running on the adapter, until [`Self::stop`].
pub struct Scan {
    adapter: Adapter,
    address: Option<Address>,
}

impl Scan {
    /// The feeders advertising setup now, as far as the adapter has heard.
    /// A feeder shows up again on each read while it is heard.
    pub async fn heard(&self) -> Result<Vec<Heard>, Error> {
        let service = Uuid::from_u128(SERVICE);
        let mut heard = Vec::new();
        for peripheral in self.adapter.peripherals().await.map_err(Error::Scan)? {
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
            let advertising = properties.rssi.is_some()
                || (self.address.is_some()
                    && peripheral.is_connected().await.map_err(Error::Scan)?);
            if advertising {
                heard.push(Heard::new(peripheral, Some(&properties)));
            }
        }
        Ok(heard)
    }

    /// Stop scanning. A stack that fails to stop has nothing more to say.
    pub async fn stop(self) {
        let _ = self.adapter.stop_scan().await;
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

    /// The stack's own name for it, which [`Radio::heard_as`] takes back:
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

/// One connected feeder's control characteristic.
pub struct Connection {
    peripheral: Peripheral,
    control: Characteristic,
}

impl Connection {
    /// Hang up, waiting at most [`CLOSE_TIMEOUT`].
    pub async fn disconnect(&self) {
        let _ = timeout(CLOSE_TIMEOUT, self.peripheral.disconnect()).await;
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
