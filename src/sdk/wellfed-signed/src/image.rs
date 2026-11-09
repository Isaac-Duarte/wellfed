//! How long the image in a bank is, from its own headers, so the feeder
//! can hash exactly the image the factory loader maps.
//!
//! A bank holds five core images back to back, each a 32-byte header (the
//! loader marker, the body's length, its load address and four erased
//! words) and its body: KM0's XIP and RAM parts, then, from the next 4 KiB
//! boundary, KM4's XIP, RAM and PSRAM parts. The image ends at the 4 KiB
//! boundary after the last body. This is the one profile the reviewed
//! builds and stock 1.1.15 use; a bank that does not match it has no
//! extent, so nothing trusts it. The host's image checks
//! (`wellfed_image::ota::images`) walk the same [`PARTS`].

use super::{MARKER, SECTOR_BYTES};

/// One core image in a bank: the load address its header names, and
/// whether it starts at a sector boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Part {
    pub address: u32,
    pub aligned: bool,
}

/// Each part, in bank order: KM0's XIP and RAM parts, then KM4's XIP part
/// from the next sector, its RAM part and its PSRAM part, the last, which
/// is always empty.
pub const PARTS: [Part; 5] = [
    Part {
        address: 0x0C00_0020,
        aligned: false,
    },
    Part {
        address: 0x0008_3000,
        aligned: false,
    },
    Part {
        address: 0x0E00_0020,
        aligned: true,
    },
    Part {
        address: 0x1000_5000,
        aligned: false,
    },
    Part {
        address: 0x0200_0020,
        aligned: false,
    },
];
/// Each part's header: the loader marker, the body's length, the load
/// address and four erased words.
pub const HEADER: u32 = 32;

fn word(header: &[u8; 32], index: usize) -> u32 {
    let at = index * 4;
    u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
}

/// The image's length in bytes, at most `limit`, reading 32-byte headers at
/// bank offsets through `read`. `None` when the headers do not match the
/// profile or the image would pass `limit`.
pub fn extent(mut read: impl FnMut(u32, &mut [u8; 32]) -> bool, limit: u32) -> Option<u32> {
    let mut offset = 0u32;
    for part in PARTS {
        if part.aligned {
            offset = offset.checked_next_multiple_of(SECTOR_BYTES)?;
        }
        let mut header = [0u8; 32];
        if offset.checked_add(HEADER)? > limit || !read(offset, &mut header) {
            return None;
        }
        let erased = (4..8).all(|i| word(&header, i) == u32::MAX);
        if header[..8] != MARKER || word(&header, 3) != part.address || !erased {
            return None;
        }

        let body = word(&header, 2);
        // Only the PSRAM part is empty, and it always is.
        if (part.address == PARTS[4].address) != (body == 0) {
            return None;
        }
        offset = offset.checked_add(HEADER)?.checked_add(body)?;
    }

    let end = offset.checked_next_multiple_of(SECTOR_BYTES)?;
    (end <= limit).then_some(end)
}
