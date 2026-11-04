//! [`Ble`], setup mode's BLE link in the simulator: the phone's side is the
//! test's, the device's the loop's, as the firmware's driver hands it whole
//! messages. A pair of message queues, with no framing or radio.

use std::collections::VecDeque;

use wellfed_device::io::BleLink;

/// Setup mode's BLE link.
#[derive(Debug, Default)]
pub struct Ble {
    advertising: bool,
    connected: bool,
    /// What the loop has not been told yet: a host connected or left.
    link: Option<BleLink>,
    /// The phone's messages, not yet taken by the loop.
    from_host: VecDeque<Vec<u8>>,
    /// The message the loop holds until it consumes it.
    held: Option<Vec<u8>>,
    /// The device's notifications, not yet read by the phone.
    to_host: VecDeque<Vec<u8>>,
}

impl Ble {
    /// Whether setup mode advertises, so a phone can connect.
    pub fn advertising(&self) -> bool {
        self.advertising
    }

    /// A phone connects and subscribes, as the firmware's driver reports
    /// both; false while nothing advertises or a phone is connected.
    pub fn connect(&mut self) -> bool {
        if !self.advertising || self.connected {
            return false;
        }
        self.connected = true;
        self.link = Some(BleLink::Opened);
        true
    }

    /// The connected phone writes one whole message.
    pub fn write(&mut self, message: &[u8]) {
        if self.connected {
            self.from_host.push_back(message.to_vec());
        }
    }

    /// What the device notified since the last call, one message each.
    pub fn notified(&mut self) -> Vec<Vec<u8>> {
        self.to_host.drain(..).collect()
    }

    /// The loop's step: a connection or a leave first, then the phone's
    /// next message once the loop has consumed the one it holds, as the
    /// driver reports them. Advertises while `advertise` and no phone is
    /// connected.
    pub fn step(&mut self, advertise: bool) -> Option<BleLink> {
        self.advertising = advertise && !self.connected;
        self.link.take().or_else(|| {
            if self.held.is_some() {
                return None;
            }
            self.held = Some(self.from_host.pop_front()?);
            Some(BleLink::Message)
        })
    }

    /// The message the loop holds.
    pub fn message(&self) -> Option<&[u8]> {
        self.held.as_deref()
    }

    pub fn consume(&mut self) {
        self.held = None;
    }

    /// Notify the phone; false with none connected.
    pub fn send(&mut self, body: &[u8]) -> bool {
        if self.connected {
            self.to_host.push_back(body.to_vec());
        }
        self.connected
    }

    /// Drop the phone: its messages go, and the loop hears it left.
    pub fn close(&mut self) {
        if self.connected {
            self.connected = false;
            self.from_host.clear();
            self.held = None;
            self.link = Some(BleLink::Closed);
        }
    }
}
