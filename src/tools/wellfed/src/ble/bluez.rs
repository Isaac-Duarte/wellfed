//! Linux's half of [`crate::ble`]: BlueZ, through `bluer`.
//!
//! BlueZ quirks met on the bench shape this: a filtered discovery
//! crashed bluetoothd 5.87, so discovery is unfiltered and the service UUID
//! is checked here; and `notify()` can miss a notification sent as soon as
//! they are enabled, so the socket from `notify_io()` is held instead.

use std::time::Duration;

use anyhow::{Context, Result, bail};
pub(crate) use bluer::Address;
use bluer::Uuid;
use bluer::gatt::WriteOp;
use bluer::gatt::remote::{Characteristic, CharacteristicWriteRequest};
use tokio::time::timeout;
use wellfed_client::setup::{CONTROL, SERVICE};

use super::{CONNECT_TIMEOUT, FIND_TIMEOUT, NOT_FOUND, Notifications};

fn uuid(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

/// One connected feeder's control characteristic.
pub(super) struct Connection {
    device: bluer::Device,
    control: Characteristic,
}

impl Connection {
    pub(super) async fn open(address: Option<Address>) -> Result<(Self, Notifications)> {
        let session = bluer::Session::new().await.context("connect to BlueZ")?;
        let adapter = session.default_adapter().await?;
        adapter.set_powered(true).await?;
        let device = find(&adapter, address).await?;
        eprintln!("Found a feeder at {}; connecting…", device.address());
        if !device.is_connected().await? {
            timeout(CONNECT_TIMEOUT, device.connect())
                .await
                .context("connecting timed out")?
                .context("connect to the feeder")?;
        }

        let control = timeout(CONNECT_TIMEOUT, control(&device))
            .await
            .context("the feeder's services did not resolve")??;
        let reader = timeout(CONNECT_TIMEOUT, control.notify_io())
            .await
            .context("enabling notifications timed out")??;
        let notifications = futures_util::stream::unfold(reader, |reader| async move {
            let notification = reader.recv().await.ok()?;
            Some((notification, reader))
        });
        Ok((Self { device, control }, Box::pin(notifications)))
    }

    /// A write request, so the piece is acknowledged and arrives in order.
    pub(super) async fn write(&self, piece: &[u8]) -> Result<()> {
        let write = CharacteristicWriteRequest {
            op_type: WriteOp::Request,
            ..Default::default()
        };
        self.control.write_ext(piece, &write).await?;
        Ok(())
    }

    pub(super) async fn disconnect(&self, wait: Duration) {
        let _ = timeout(wait, self.device.disconnect()).await;
    }
}

/// The feeder advertising the setup service now: `address` if given,
/// otherwise the first one heard. Discovery runs even for a known address:
/// BlueZ connects only to a device it has heard since it last forgot it,
/// and refuses one it has not ("not available").
async fn find(adapter: &bluer::Adapter, address: Option<Address>) -> Result<bluer::Device> {
    // Held for the search: discovery runs while the stream lives.
    let _discovery = adapter.discover_devices().await?;
    timeout(FIND_TIMEOUT, async {
        loop {
            // BlueZ reports a cached device once, before it has been heard,
            // so poll the list: only a device advertising now has an RSSI.
            for heard in adapter.device_addresses().await? {
                if address.is_some_and(|wanted| wanted != heard) {
                    continue;
                }
                let device = adapter.device(heard)?;
                // A feeder still linked from a run before stops
                // advertising; the address names it, so it counts.
                let advertising = device.rssi().await?.is_some()
                    || (address.is_some() && device.is_connected().await?);
                let setup = device
                    .uuids()
                    .await?
                    .is_some_and(|uuids| uuids.contains(&uuid(SERVICE)));
                if advertising && setup {
                    return Ok::<_, anyhow::Error>(device);
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await
    .context(NOT_FOUND)?
}

async fn control(device: &bluer::Device) -> Result<Characteristic> {
    for service in device.services().await? {
        if service.uuid().await? != uuid(SERVICE) {
            continue;
        }
        for characteristic in service.characteristics().await? {
            if characteristic.uuid().await?.as_u128() == CONTROL {
                return Ok(characteristic);
            }
        }
    }
    bail!("the device has no setup service")
}
