//! The signed formats bank 0 checks, frozen for good: the version 2
//! manifest, the downgrade, key-add and revoke statements, the Ed25519
//! signatures over them and the key ids that name their signers, and the
//! bank limits and loader layout an offer's image is checked against.
//!
//! Every update passes through bank 0, which keeps the build it was written
//! with. A client or signer that drifted from these bytes could never update
//! that feeder again without a UART cable, so nothing here changes in place:
//! only a new major of the device API's update path changes the frozen part
//! ([device API](../../../../docs/firmware/device-api.md#the-frozen-part)).
//! `tests/signed_formats.rs` pins every format against
//! `tests/fixtures/signed-v1.txt`, whose lines are only ever appended.
//!
//! The crate holds exactly the frozen set and takes no other workspace
//! crate, so the firmware, clients and the server all compile the one small
//! verifier. Everything that may change (the sector writer, the boot trial,
//! the confirmation record and bank 0's install) lives in `wellfed-core`.
#![no_std]
#![forbid(unsafe_code)]

pub mod image;
pub mod offer;

/// The retired version 1 manifest (148 bytes), which builds fetching
/// updates over MQTT took. An offer refuses its magic, and a signer still
/// reads the versions of the ones on disk.
pub(crate) const RETIRED_MAGIC: [u8; 8] = *b"PLOTA1\0\0";
const RETIRED_LEN: usize = 148;

/// One NOR erase sector, as flash offsets measure it and as buffers do.
pub const SECTOR_BYTES: u32 = 0x1000;
pub const SECTOR: usize = SECTOR_BYTES as usize;

/// The factory loader's validity marker at the start of a bank.
pub const MARKER: [u8; 8] = *b"81958711";
/// Bank 1 starts at 0x186000; the reviewed footprint ends at 0x2d6000.
pub const BANK1_OFFSET: u32 = 0x18_6000;
pub const BANK1_BYTES: u32 = 0x15_0000;

/// Whether an image of `len` bytes is one bank 1 takes: at least a sector,
/// and no more than the bank. The firmware's offer check and the signer
/// both ask this, so a build the signer passes is one the feeder admits.
pub const fn image_len_fits(len: u32) -> bool {
    len >= SECTOR_BYTES && len <= BANK1_BYTES
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub version: u32,
    pub image_len: u32,
    pub target_bank: u32,
    pub image_sha256: [u8; 32],
    pub build_id: [u8; 32],
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// The version a manifest's header claims, unverified: `None` unless it has
/// a version 2 or retired version 1 manifest's length and magic (both keep
/// the version at byte 8). For a signer picking the next version above the
/// ones it already signed; a feeder only trusts `offer::admit_offer`.
pub fn claimed_version(bytes: &[u8]) -> Option<u32> {
    let v1 = bytes.len() == RETIRED_LEN && bytes[..8] == RETIRED_MAGIC;
    let v2 = bytes.len() == offer::MANIFEST_LEN && bytes[..8] == offer::MAGIC;
    (v1 || v2).then(|| u32_at(bytes, 8))
}
