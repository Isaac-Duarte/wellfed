//! The two flash banks an image is installed into, and the payload of an
//! image that fits one.
//!
//! Bank 1 (`0x186000`) is the factory loader's preferred bank; bank 0
//! (`0x6000`) is its fallback. Each keeps the reviewed stock footprint. The
//! factory region `0x0-0x6000` belongs to neither.

use wellfed_formats::flash_plan::Kind;
use wellfed_signed::SECTOR;

use wellfed_formats::require;

use crate::error::Result;

/// A bank this layout does not have, or an image that does not fit one.
#[derive(Debug, thiserror::Error)]
pub enum BankError {
    #[error("Unknown bank")]
    Unknown { bank: u8 },
    #[error("Development image exceeds aligned bank-{bank} footprint")]
    Footprint { bank: u8 },
}

/// One bank's region, and the other bank's first sector read as a guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub kind: Kind,
    pub bank: u8,
    pub start: usize,
    pub end: usize,
    pub other: (usize, usize),
}

pub const BANK1: Layout = Layout {
    kind: Kind::Bank1Update,
    bank: 1,
    start: 0x186000,
    end: 0x2D6000,
    other: (0x6000, SECTOR),
};

/// Bank 0 keeps the reviewed stock footprint (0x150000), like bank 1.
pub const BANK0: Layout = Layout {
    kind: Kind::Bank0Update,
    bank: 0,
    start: 0x6000,
    end: 0x156000,
    other: (BANK1.start, SECTOR),
};

/// The PLWF106 fountain's bank 1, the only bank a fountain candidate goes
/// into: bank 0 keeps the stock image its loader falls back to. The window
/// is stock bank 0's rounded footprint (`0xeb000`), a conservative cap
/// rather than the bank's capacity; the fountain's `layout.json` records it.
/// Its guard sector is bank 0's first.
pub const PLWF106_BANK1: Layout = Layout {
    kind: Kind::FountainBank1Update,
    bank: 1,
    start: 0x186000,
    end: 0x271000,
    other: (0x6000, SECTOR),
};

pub fn layout(bank: u8) -> Result<Layout> {
    match bank {
        0 => Ok(BANK0),
        1 => Ok(BANK1),
        _ => Err(BankError::Unknown { bank }.into()),
    }
}

/// The aligned application payload of a complete OTA image for this bank.
pub fn payload<'a>(image: &'a [u8], layout: &Layout) -> Result<&'a [u8]> {
    let data = crate::ota::container(image)?;
    require(
        !data.is_empty() && data.len() <= layout.end - layout.start && data.len() % SECTOR == 0,
        || BankError::Footprint { bank: layout.bank },
    )?;
    crate::ota::images(data)?;
    Ok(data)
}
