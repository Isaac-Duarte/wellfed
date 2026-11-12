//! Where a flash write or read may go, and how a program splits into NOR
//! pages: one rule both backends apply. The target adds each span to its
//! region's flash offset and programs page by page; the fake indexes its
//! modelled regions with the same span, so every host test of a writer runs
//! the bounds the hardware build uses.

use wellfed_board::STORAGE_SECTORS;

use crate::{BANK0_OFFSET, BANK1_OFFSET, BANK1_SECTORS, SECTOR_BYTES};

/// A NOR program page: one program never crosses its boundary.
pub(crate) const PROGRAM_PAGE: u32 = 256;

/// `len` bytes at `offset` inside one sector: the sector-relative end, if
/// it is no further than the sector's last byte.
fn within_sector(offset: u32, len: usize) -> bool {
    u32::try_from(len)
        .ok()
        .and_then(|len| offset.checked_add(len))
        .is_some_and(|end| end <= SECTOR_BYTES)
}

/// The offset from the storage region's start of `len` bytes at `offset` in
/// storage sector `sector`, if they lie inside it. On a board with no store
/// every sector is refused (a range, since `sector < 0` would be a
/// comparison that is always false on that board).
pub(crate) fn storage(sector: u32, offset: u32, len: usize) -> Option<u32> {
    ((0..STORAGE_SECTORS).contains(&sector) && within_sector(offset, len))
        .then(|| sector * SECTOR_BYTES + offset)
}

/// The offset from bank 1's start of `len` bytes at `offset` in bank-1
/// sector `sector`, only while `running` from bank 0: the updater never
/// writes the bank it executes from.
pub(crate) fn bank1(running: Option<u8>, sector: u32, offset: u32, len: usize) -> Option<u32> {
    (running == Some(0) && sector < BANK1_SECTORS && within_sector(offset, len))
        .then(|| sector * SECTOR_BYTES + offset)
}

/// `len` bytes at `offset` from bank 0's start, if they end inside bank 0.
pub(crate) fn bank0(offset: u32, len: usize) -> Option<u32> {
    let end = offset.checked_add(u32::try_from(len).ok()?)?;
    (end <= BANK1_OFFSET - BANK0_OFFSET).then_some(offset)
}

/// A program's offset and length are both whole words.
pub(crate) fn aligned(offset: u32, len: usize) -> bool {
    offset.is_multiple_of(4) && len.is_multiple_of(4)
}

/// One NOR program: `len` bytes at flash `address`, taken from the data
/// from `from`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Program {
    pub(crate) address: u32,
    pub(crate) from: usize,
    pub(crate) len: u32,
}

/// The programs that write `len` bytes from flash `address` on, split so no
/// program crosses a [`PROGRAM_PAGE`] boundary. `len` comes from a span
/// already checked, so it fits a sector.
pub(crate) fn programs(address: u32, len: u32) -> impl Iterator<Item = Program> {
    let mut done = 0u32;
    core::iter::from_fn(move || {
        if done >= len {
            return None;
        }
        let at = address.wrapping_add(done);
        let room = PROGRAM_PAGE - at % PROGRAM_PAGE;
        let program = Program {
            address: at,
            from: done as usize,
            len: room.min(len - done),
        };
        done += program.len;
        Some(program)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wellfed_board::STORAGE_OFFSET;

    #[test]
    fn programs_cover_the_span_and_never_cross_a_page() {
        for address in [0, 4, 252, 256, 0x18_6000, 0x18_60fc, STORAGE_OFFSET + 0x1f0] {
            for len in [0, 4, 8, 252, 256, 260, 512, 1020, SECTOR_BYTES] {
                let mut next = address;
                let mut from = 0;
                for program in programs(address, len) {
                    assert_eq!((program.address, program.from), (next, from));
                    assert!(program.len > 0 && program.len <= PROGRAM_PAGE);
                    let last = program.address + program.len - 1;
                    assert_eq!(program.address / PROGRAM_PAGE, last / PROGRAM_PAGE);
                    next += program.len;
                    from += program.len as usize;
                }
                assert_eq!(next, address + len, "{address:#x} + {len}");
            }
        }
    }

    #[test]
    fn spans_stay_inside_their_sector_and_region() {
        let sector = SECTOR_BYTES as usize;
        assert_eq!(storage(0, 0, sector), Some(0));
        assert_eq!(storage(1, 8, 4), Some(SECTOR_BYTES + 8));
        assert_eq!(storage(0, 4, sector), None, "past the sector's end");
        assert_eq!(storage(0, u32::MAX, 4), None, "an offset that wraps");
        assert_eq!(storage(0, 0, usize::MAX), None, "a length past u32");
        assert_eq!(storage(STORAGE_SECTORS, 0, 4), None, "past the store");

        assert_eq!(bank1(Some(0), 0, 0, sector), Some(0));
        assert_eq!(
            bank1(Some(0), BANK1_SECTORS - 1, 0, 4),
            Some((BANK1_SECTORS - 1) * SECTOR_BYTES)
        );
        assert_eq!(bank1(Some(0), BANK1_SECTORS, 0, 4), None, "past bank 1");
        assert_eq!(bank1(Some(1), 0, 0, 4), None, "running from bank 1");
        assert_eq!(bank1(None, 0, 0, 4), None, "an unknown bank");
        assert_eq!(bank1(Some(0), 0, 4, sector), None, "past the sector's end");

        let bank0_bytes = (BANK1_OFFSET - BANK0_OFFSET) as usize;
        assert_eq!(bank0(0, bank0_bytes), Some(0));
        assert_eq!(bank0(1, bank0_bytes), None, "past bank 0");
        assert_eq!(bank0(u32::MAX, 1), None);

        assert!(aligned(8, 12) && !aligned(2, 4) && !aligned(4, 6));
    }

    /// Every flash byte a span reaches, at every sector and offset, lies in
    /// its own region: the store, bank 1 or bank 0, never another.
    #[test]
    fn every_span_lands_in_its_region() {
        let sector = SECTOR_BYTES as usize;
        for index in 0..STORAGE_SECTORS {
            let span = storage(index, 0, sector).unwrap();
            assert!(span + SECTOR_BYTES <= STORAGE_SECTORS * SECTOR_BYTES);
        }
        for index in 0..BANK1_SECTORS {
            let span = bank1(Some(0), index, 0, sector).unwrap();
            let end = BANK1_OFFSET + span + SECTOR_BYTES;
            assert!(
                BANK1_OFFSET + span >= BANK1_OFFSET
                    && end <= BANK1_OFFSET + BANK1_SECTORS * SECTOR_BYTES
            );
        }
    }
}
