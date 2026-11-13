//! A feeder named on the command line. macOS never shows a device's
//! Bluetooth address: CoreBluetooth names each one by a UUID of its own,
//! which differs from one Mac to the next, so that UUID is what an address
//! is there. Linux and Windows take the Bluetooth address.

use std::fmt;
use std::str::FromStr;

#[cfg(not(target_os = "macos"))]
use btleplug::api::BDAddr;
use btleplug::api::Peripheral as _;
use btleplug::platform::Peripheral;
#[cfg(target_os = "macos")]
use uuid::Uuid;

/// One feeder, as this computer's Bluetooth stack names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Address(
    #[cfg(target_os = "macos")] Uuid,
    #[cfg(not(target_os = "macos"))] BDAddr,
);

impl Address {
    /// Whether `peripheral` is the feeder this names.
    pub(crate) fn names(self, peripheral: &Peripheral) -> bool {
        #[cfg(target_os = "macos")]
        let named = peripheral.id().to_string() == self.0.to_string();
        #[cfg(not(target_os = "macos"))]
        let named = peripheral.address() == self.0;
        named
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for Address {
    type Err = AddressError;

    #[cfg(target_os = "macos")]
    fn from_str(text: &str) -> Result<Self, AddressError> {
        Uuid::parse_str(text)
            .map(Self)
            .map_err(|_| AddressError::NotUuid)
    }

    #[cfg(not(target_os = "macos"))]
    fn from_str(text: &str) -> Result<Self, AddressError> {
        text.parse()
            .map(Self)
            .map_err(|_| AddressError::NotBluetooth)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AddressError {
    #[error(
        "macOS names a Bluetooth device by a UUID, not by its address; leave this out to take the first feeder in setup mode"
    )]
    NotUuid,
    #[error("a Bluetooth address is six hex bytes, as 00:11:22:33:44:55")]
    NotBluetooth,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
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

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn an_address_is_a_bluetooth_address_here() {
        let address = "00:11:22:33:44:5a".parse::<Address>().unwrap();
        assert_eq!(address.to_string().to_lowercase(), "00:11:22:33:44:5a");

        let refused = "4c5a2d3e-8f10-4b6a-9c7d-0e1f2a3b4c5d"
            .parse::<Address>()
            .unwrap_err();
        assert!(matches!(refused, AddressError::NotBluetooth));
    }
}
