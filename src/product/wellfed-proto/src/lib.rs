//! The device protocol's messages ([API design]): what a device and its
//! clients say to each other once a Noise session carries their bytes.
//!
//! Messages are postcard-encoded serde types, so the device (no allocation),
//! the host client and the app's generated bindings read one definition.
//! Nothing here does IO or cryptography: the caller seals and opens the bytes.
//! The BLE setup exchange ([`setup`]) and the device API ([`api`])
//! share `wellfed_session::frame`'s length-prefixed frames.
//! A feed's trace body ([`trace`]) is a compact format of its own,
//! carried in the device API's pages and kept in the feeder's storage.
//! What a device is, its kind and model ([`device`]), opens every session.
//! Who may send which op ([`access`]) is the device's rule and every
//! client's gate.
//! [`grow`] holds what lets a message grow without stranding older readers.
//!
//! [API design]: ../../../../docs/firmware/device-api.md
#![no_std]

#[macro_use]
pub mod grow;
pub mod access;
pub mod api;
pub mod device;
pub mod setup;
pub mod trace;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("the output buffer cannot hold the message")]
    BufferTooSmall,
    #[error("the bytes are not one well-formed message")]
    Malformed,
}

/// Encode `value` into `out`; the encoded length.
pub fn encode<T: Serialize>(value: &T, out: &mut [u8]) -> Result<usize, Error> {
    postcard::to_slice(value, out)
        .map(|used| used.len())
        .map_err(|error| match error {
            postcard::Error::SerializeBufferFull => Error::BufferTooSmall,
            _ => Error::Malformed,
        })
}

/// Decode exactly one message: trailing bytes are malformed, not ignored.
pub fn decode<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, Error> {
    match postcard::take_from_bytes(bytes) {
        Ok((value, [])) => Ok(value),
        _ => Err(Error::Malformed),
    }
}

/// Decode one message from the start of `bytes`, whose rest must be zeros:
/// a fixed-size record field (the event ring's 16-byte body).
///
/// Only bytes the value encodes back to are a record. Postcard reads an
/// overlong varint (`0x96 0x00` for 22) as the value it ends on, which no
/// build ever writes, so such a body would read as an event whose bytes
/// differ from the ones stored, hashed and passed on. A body whose
/// [`Lenient`](grow::Lenient) index is 256 or more is refused too: it reads
/// as `Unknown` saturated to 255 and so encodes back to other bytes, and no
/// unit enum comes near that many variants.
pub fn decode_padded<'a, T: Deserialize<'a> + Serialize>(bytes: &'a [u8]) -> Option<T> {
    let (value, rest) = postcard::take_from_bytes::<T>(bytes).ok()?;
    let read = bytes.get(..bytes.len() - rest.len())?;
    let canonical = postcard::serialize_with_flavor(
        &value,
        Same {
            expected: read,
            at: 0,
        },
    );
    (rest.iter().all(|&b| b == 0) && canonical == Ok(true)).then_some(value)
}

/// A postcard output that compares each byte with the next of `expected`
/// instead of keeping it, so checking an encoding needs no buffer.
struct Same<'a> {
    expected: &'a [u8],
    at: usize,
}

impl postcard::ser_flavors::Flavor for Same<'_> {
    /// Whether the encoding was all of `expected`.
    type Output = bool;

    fn try_push(&mut self, byte: u8) -> postcard::Result<()> {
        if self.expected.get(self.at) != Some(&byte) {
            return Err(postcard::Error::SerializeBufferFull);
        }
        self.at += 1;
        Ok(())
    }

    fn finalize(self) -> postcard::Result<bool> {
        Ok(self.at == self.expected.len())
    }
}
