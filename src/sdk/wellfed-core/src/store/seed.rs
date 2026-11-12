//! The random generator's seed, carried across boots.
//!
//! A 128-byte record in the dual-slot store ([`slot`](crate::store::slot)):
//! magic `PLFSEED1`, version, record size, generation, the 32-byte seed,
//! the event ring's epoch (minor 1), zero padding, CRC-32/ISO-HDLC of bytes
//! 0..120 at 120 and `CMIT` at 124.
//! The newest valid record wins and a torn newer slot falls back to the
//! older one. A newer minor is read for the seed and rewritten with the
//! rest kept; a newer major makes the store read-only. With no valid record
//! the store starts empty and writable whatever its units hold, an
//! unreadable record of its own included: the seed is only a nonce mixed
//! with fresh input, so writing over one this build cannot read costs
//! nothing, and holding back would leave every later boot without it.
//!
//! [`start`] is the only intended writer: each boot instantiates the DRBG
//! from fresh pool input with the stored seed as its nonce, then stores a
//! new seed drawn from it before any key or nonce is handed out. A boot
//! whose fresh input repeats a previous one still starts from a seed no
//! earlier boot used. The seed sits in plain flash, as every secret on this
//! board does; reading it lets an attacker predict nothing once a boot has
//! mixed fresh input in.
//!
//! **The ring's epoch.** The record also carries the epoch an event ring
//! created this boot takes. The seed is written every boot and kept through
//! a factory reset, which erases the ring, so a writer that moves the epoch
//! on with each boot's seed gives every ring a number no earlier one had,
//! at no extra write. 0 is none yet.

use core::ffi::c_uint;

use crate::rng::{Drbg, Error};
use crate::store::slot::{
    CRC_AT, DualSlot, Extent, Layout, Newest, RECORD_BYTES, SlotIo, Version, get_u32, get_u64,
    put_u32, put_u64, seal,
};

pub const VERSION: Version = Version::new(1, 1);
const LAYOUT: Layout = Layout::new(*b"PLFSEED1", VERSION, EPOCH_END, Extent::Record);
const _: () = assert!(LAYOUT.fits(RECORD_BYTES));
const GENERATION_AT: usize = 12;
const SEED_AT: usize = 20;
const SEED_END: usize = SEED_AT + 32;
const EPOCH_AT: usize = SEED_END;
const EPOCH_END: usize = EPOCH_AT + 4;
/// Personalization for the firmware's DRBG.
const PERSONALIZATION: &[u8] = b"wellfed-fw45-drbg";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeedStatus {
    Unopened,
    /// No seed yet (a first boot, or every save was cut short).
    Empty,
    Loaded,
    /// A newer schema version: read-only.
    ReadOnly,
    Io,
}

#[derive(Clone, Copy)]
pub struct SeedStore<I> {
    units: DualSlot<I>,
    pub status: SeedStatus,
    pub generation: u64,
    pub slot: c_uint,
    seed: Option<[u8; 32]>,
    epoch: u32,
}

impl<I> core::fmt::Debug for SeedStore<I> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SeedStore")
            .field("status", &self.status)
            .field("generation", &self.generation)
            .field("slot", &self.slot)
            .finish_non_exhaustive()
    }
}

impl<I> Default for SeedStore<I> {
    fn default() -> Self {
        Self {
            units: DualSlot::unbound(&LAYOUT),
            status: SeedStatus::Unopened,
            generation: 0,
            slot: 0,
            seed: None,
            epoch: 0,
        }
    }
}

fn encode(generation: u64, seed: &[u8; 32], epoch: u32) -> [u8; RECORD_BYTES] {
    let mut p = LAYOUT.record();
    put_u64::<GENERATION_AT, _>(&mut p, generation);
    p[SEED_AT..SEED_END].copy_from_slice(seed);
    put_u32::<EPOCH_AT, _>(&mut p, epoch);
    seal(&mut p);
    p
}

/// The generation, the seed and the ring's epoch.
fn decode(p: &[u8; RECORD_BYTES]) -> Option<(u64, ([u8; 32], u32))> {
    let generation = get_u64::<GENERATION_AT, _>(p);
    if generation == 0 || p[EPOCH_END..CRC_AT].iter().any(|&b| b != 0) {
        return None;
    }
    let mut seed = [0; 32];
    seed.copy_from_slice(&p[SEED_AT..SEED_END]);
    Some((generation, (seed, get_u32::<EPOCH_AT, _>(p))))
}

impl<I: SlotIo> SeedStore<I> {
    /// Bind to two exclusive erase units and load the newest valid seed.
    pub fn open(&mut self, io: I) -> SeedStatus {
        *self = Self::default();
        self.status = match self.units.open_newest(io, decode) {
            Newest::Io => SeedStatus::Io,
            Newest::Future => SeedStatus::ReadOnly,
            Newest::Loaded {
                slot,
                generation,
                value: (seed, epoch),
            } => {
                self.slot = slot;
                self.generation = generation;
                self.seed = Some(seed);
                self.epoch = epoch;
                SeedStatus::Loaded
            }
            Newest::Nothing { .. } => SeedStatus::Empty,
        };
        self.status
    }

    pub fn seed(&self) -> Option<&[u8; 32]> {
        self.seed.as_ref()
    }

    /// The event ring's epoch last stored; 0 before any.
    pub fn epoch(&self) -> u32 {
        self.epoch
    }

    pub fn writable(&self) -> bool {
        matches!(self.status, SeedStatus::Empty | SeedStatus::Loaded)
    }

    /// Store `seed` and the ring's `epoch` to the other slot by the
    /// torn-write rule. A failure keeps the previous ones in memory.
    pub fn save(&mut self, seed: &[u8; 32], epoch: u32) -> bool {
        if !self.writable() || !self.units.writable() {
            return false;
        }
        let saved = self
            .units
            .save_newest(self.generation, self.slot, |generation| {
                encode(generation, seed, epoch)
            });
        let Some((slot, generation)) = saved else {
            self.status = SeedStatus::Io;
            return false;
        };
        self.slot = slot;
        self.generation = generation;
        self.seed = Some(*seed);
        self.epoch = epoch;
        true
    }
}

/// The boot's generator: `fresh` pool input with the stored seed, if any,
/// as the nonce; then the next boot's seed is stored. The caller hands out
/// nothing before this returns.
///
/// A failed save still returns the generator, since `fresh` is health-tested
/// input of full claimed entropy; the store's status reports the failure.
pub fn start<I: SlotIo>(store: &mut SeedStore<I>, fresh: &[u8; 32]) -> Result<Drbg, Error> {
    let stored = store.seed.unwrap_or([0; 32]);
    let mut drbg = Drbg::new(fresh, &stored, PERSONALIZATION);
    let next = drbg.bytes()?;
    store.save(&next, store.epoch);
    Ok(drbg)
}
