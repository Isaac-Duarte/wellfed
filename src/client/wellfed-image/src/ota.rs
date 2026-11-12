//! The known PLAF103 OTA container and region profile.
//!
//! Development updates and frozen reviews take a bank's payload only through
//! `container` and `regions`, so an image outside this one profile is refused.

use serde::Serialize;
use wellfed_formats::require;
use wellfed_signed::image::{HEADER, PARTS};
use wellfed_signed::{MARKER, SECTOR};

use crate::error::Result;

/// A container or payload outside the one profile the factory loader is
/// known to accept.
#[derive(Debug, thiserror::Error)]
pub enum OtaError {
    #[error("OTA header is truncated")]
    Truncated,
    #[error("Unsupported OTA container profile")]
    Profile,
    #[error("OTA payload size does not match EOF")]
    Size,
    #[error("OTA byte-sum checksum mismatch")]
    Checksum,
    #[error("Unexpected inter-core padding")]
    InterCorePadding,
    #[error("Missing {name} image header")]
    MissingHeader { name: &'static str },
    #[error("Unsupported {name} header/load address")]
    Header { name: &'static str },
    #[error("Unsupported {name} body size")]
    BodySize { name: &'static str },
    #[error("{name} body exceeds payload")]
    BodyOverrun { name: &'static str },
    #[error("Unexpected final padding/extra regions")]
    FinalPadding,
}

/// The original stock image, the only one a frozen review may accept
/// without a build manifest.
pub const STOCK_SHA256: &str = "05147476fc70c91cc77ad7da6d6772376b2081d460ac7b1912e2d73868a09498";
/// What a report calls each of the loader's [`PARTS`], in bank order.
const NAMES: [&str; PARTS.len()] = ["km0_xip", "km0_ram", "km4_xip", "km4_ram", "km4_psram"];

fn words(data: &[u8], at: usize) -> [u32; 8] {
    std::array::from_fn(|i| {
        let word = at + i * 4;
        u32::from_le_bytes([data[word], data[word + 1], data[word + 2], data[word + 3]])
    })
}

/// The application payload of a single-entry OTA container.
pub fn container(data: &[u8]) -> Result<&[u8]> {
    require(data.len() >= 32, || OtaError::Truncated)?;
    let [
        version,
        count,
        magic,
        entry,
        checksum,
        size,
        offset,
        address,
    ] = words(data, 0);

    require(
        (version, count, magic, entry, offset, address)
            == (
                0xffff_ffff,
                1,
                u32::from_le_bytes(*b"OTA1"),
                24,
                32,
                0xffff_ffff,
            ),
        || OtaError::Profile,
    )?;
    require(size > 0 && size as usize == data.len() - 32, || {
        OtaError::Size
    })?;

    let payload = &data[32..];
    let sum = payload
        .iter()
        .fold(0u32, |sum, byte| sum.wrapping_add(*byte as u32));
    require(sum == checksum, || OtaError::Checksum)?;

    Ok(payload)
}

/// One core image in a bank payload, as the layout audit lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Image {
    pub name: &'static str,
    pub payload_header_offset: usize,
    pub body_bytes: usize,
    pub runtime_address: String,
}

/// Both cores' images at their exact addresses, with only erased padding:
/// the loader's one profile ([`PARTS`]), with every padding byte checked.
pub fn images(payload: &[u8]) -> Result<Vec<Image>> {
    let header_len = HEADER as usize;
    let mut result = Vec::new();
    let mut offset = 0usize;

    for (index, (name, part)) in NAMES.into_iter().zip(PARTS).enumerate() {
        if part.aligned {
            let aligned = offset.next_multiple_of(SECTOR);
            require(
                aligned <= payload.len() && payload[offset..aligned].iter().all(|b| *b == 0xff),
                || OtaError::InterCorePadding,
            )?;
            offset = aligned;
        }

        require(offset + header_len <= payload.len(), || {
            OtaError::MissingHeader { name }
        })?;
        let header = words(payload, offset);
        require(
            payload[offset..offset + MARKER.len()] == MARKER
                && header[3] == part.address
                && header[4..] == [0xffff_ffff; 4],
            || OtaError::Header { name },
        )?;

        // Only the last part, KM4's PSRAM, is empty, and it always is.
        let body = header[2] as usize;
        require((index == PARTS.len() - 1) == (body == 0), || {
            OtaError::BodySize { name }
        })?;
        require(offset + header_len + body <= payload.len(), || {
            OtaError::BodyOverrun { name }
        })?;

        result.push(Image {
            name,
            payload_header_offset: offset,
            body_bytes: body,
            runtime_address: format!("0x{:08x}", part.address),
        });
        offset += header_len + body;
    }

    require(
        payload.len() == offset.next_multiple_of(SECTOR)
            && payload[offset..].iter().all(|b| *b == 0xff),
        || OtaError::FinalPadding,
    )?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{synthetic_payload, wrap};

    #[test]
    fn synthetic_images_pass_and_damage_is_refused() {
        let payload = synthetic_payload(5000, 9000, 1);
        let image = wrap(&payload);
        assert_eq!(container(&image).unwrap(), &payload[..]);
        assert_eq!(images(&payload).unwrap().len(), 5);
        assert_eq!(images(&payload).unwrap()[2].name, "km4_xip");

        let mut bad = image.clone();
        bad[40] ^= 1;
        assert!(matches!(
            container(&bad),
            Err(crate::Error::Ota(OtaError::Checksum))
        ));

        let mut padding = payload.clone();
        let last = padding.len() - 1;
        padding[last] = 0;
        assert!(matches!(
            images(&padding),
            Err(crate::Error::Ota(OtaError::FinalPadding))
        ));

        assert!(matches!(
            container(&image[..image.len() - 1]),
            Err(crate::Error::Ota(OtaError::Size))
        ));
    }
}
