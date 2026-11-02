//! macOS's half of [`crate::ble`]: CoreBluetooth, through `btleplug`.
//!
//! CoreBluetooth never shows a device's Bluetooth address. It names each one
//! by a UUID of its own, which differs from one Mac to the next, so that UUID
//! is what `--address` takes here.
//!
//! macOS asks for Bluetooth access on behalf of the app that started this
//! process, usually the terminal, under Privacy & Security in System Settings.
//!
//! This has not run against a feeder.

use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use btleplug::api::{
    Central, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures_util::StreamExt;
use tokio::time::timeout;
use uuid::Uuid;
use wellfed_client::setup::{CONTROL, SERVICE};

use super::{CONNECT_TIMEOUT, FIND_TIMEOUT, NOT_FOUND, Notifications};

const NO_ACCESS: &str = "open this Mac's Bluetooth: check it is on, and that your terminal may use it (System Settings, Privacy & Security, Bluetooth)";

/// A feeder as CoreBluetooth names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Address(Uuid);

impl FromStr for Address {
    type Err = AddressError;

    fn from_str(text: &str) -> Result<Self, AddressError> {
        Uuid::parse_str(text)
            .map(Self)
            .map_err(|_| AddressError::NotUuid)
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum AddressError {
    #[error(
        "macOS names a Bluetooth device by a UUID, not by its address; leave this out to take the first feeder in setup mode"
    )]
    NotUuid,
}

/// One connected feeder's control characteristic.
pub(super) struct Connection {
    peripheral: Peripheral,
    control: Characteristic,
}

impl Connection {
    pub(super) async fn open(address: Option<Address>) -> Result<(Self, Notifications)> {
        let manager = Manager::new().await.context(NO_ACCESS)?;
        let adapter = manager
            .adapters()
            .await
            .context(NO_ACCESS)?
            .into_iter()
            .next()
            .context(NO_ACCESS)?;

        let peripheral = find(&adapter, address).await?;
        eprintln!("Found a feeder at {}; connecting…", peripheral.id());
        timeout(CONNECT_TIMEOUT, peripheral.connect())
            .await
            .context("connecting timed out")?
            .context("connect to the feeder")?;
        timeout(CONNECT_TIMEOUT, peripheral.discover_services())
            .await
            .context("the feeder's services did not resolve")??;
        let control = control(&peripheral)?;

        // The stream first: a notification sent as soon as they are enabled
        // must have somewhere to go.
        let stream = peripheral.notifications().await?;
        timeout(CONNECT_TIMEOUT, peripheral.subscribe(&control))
            .await
            .context("enabling notifications timed out")??;

        let wanted = control.uuid;
        let notifications = stream.filter_map(move |notification| async move {
            (notification.uuid == wanted).then_some(notification.value)
        });
        Ok((
            Self {
                peripheral,
                control,
            },
            Box::pin(notifications),
        ))
    }

    /// With a response, so the piece is acknowledged and arrives in order.
    pub(super) async fn write(&self, piece: &[u8]) -> Result<()> {
        self.peripheral
            .write(&self.control, piece, WriteType::WithResponse)
            .await?;
        Ok(())
    }

    pub(super) async fn disconnect(&self, wait: Duration) {
        let _ = timeout(wait, self.peripheral.disconnect()).await;
    }
}

/// The feeder advertising the setup service: `address` if given, otherwise
/// the first one heard. The scan stops either way, as nothing else needs it.
async fn find(adapter: &Adapter, address: Option<Address>) -> Result<Peripheral> {
    let filter = ScanFilter {
        services: vec![Uuid::from_u128(SERVICE)],
    };
    adapter.start_scan(filter).await.context(NO_ACCESS)?;

    let found = timeout(FIND_TIMEOUT, heard(adapter, address)).await;
    let _ = adapter.stop_scan().await;
    found.context(NOT_FOUND)?
}

/// Poll the devices the scan has heard until one is the feeder.
async fn heard(adapter: &Adapter, address: Option<Address>) -> Result<Peripheral> {
    let service = Uuid::from_u128(SERVICE);
    let wanted = address.map(|Address(id)| id.to_string());
    loop {
        for peripheral in adapter.peripherals().await? {
            if wanted
                .as_ref()
                .is_some_and(|wanted| *wanted != peripheral.id().to_string())
            {
                continue;
            }
            let setup = peripheral
                .properties()
                .await?
                .is_some_and(|properties| properties.services.contains(&service));
            if setup {
                return Ok(peripheral);
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn control(peripheral: &Peripheral) -> Result<Characteristic> {
    let (service, control) = (Uuid::from_u128(SERVICE), Uuid::from_u128(CONTROL));
    let found = peripheral
        .characteristics()
        .into_iter()
        .find(|characteristic| {
            characteristic.service_uuid == service && characteristic.uuid == control
        });
    match found {
        Some(characteristic) => Ok(characteristic),
        None => bail!("the device has no setup service"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_a_uuid_here_and_a_bluetooth_address_says_why_not() {
        let id = "4c5a2d3e-8f10-4b6a-9c7d-0e1f2a3b4c5d";
        assert_eq!(
            id.parse::<Address>().unwrap(),
            Address(Uuid::parse_str(id).unwrap())
        );

        let refused = "00:11:22:33:44:55".parse::<Address>().unwrap_err();
        assert!(matches!(refused, AddressError::NotUuid));
    }
}
