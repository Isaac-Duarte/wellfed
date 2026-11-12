//! Wi-Fi credentials stored on NOR, set without a rebuild.
//!
//! A 128-byte record in the dual-slot store
//! (`wellfed_core::store::slot`), as the settings are: magic `PLFNET01`,
//! version, record size, generation, SSID length, passphrase length, SSID
//! (32 bytes), passphrase (64 bytes), zero padding, CRC-32/ISO-HDLC of bytes
//! 0..120 at 120 and the `CMIT` commit marker at 124.
//! A record with an empty SSID means "use the build's credentials", so a
//! clear keeps the generation rising. The newest valid record wins; a torn or
//! invalid newer slot falls back to the older one. The two sectors belong to
//! this store alone, and it is written only when the owner or setup sets
//! the network, so with no valid record it starts writable whatever it
//! holds: a record this build cannot read is replaced by the owner's choice,
//! never on the build's own. A newer minor is read for the credentials and
//! rewritten with the rest kept. Only a newer major makes the store
//! read-only; then, as with no record, the build's credentials apply.

use core::ffi::c_uint;

use wellfed_core::store::slot::{
    CRC_AT, DualSlot, Extent, Layout, Newest, RECORD_BYTES, SlotIo, Version, get_u64, put_u64, seal,
};
use wellfed_device_api::wifi::{Credentials, MAX_PSK, MAX_SSID};

/// Whether the image was built with Wi-Fi credentials of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildWifi {
    /// A bench image: the C entry prefills the bench hotspot.
    Present,
    /// A consumer image: the only network is the one the owner stores, so
    /// a boot with none stored starts BLE setup mode on its own.
    Absent,
}

pub const VERSION: Version = Version::new(1, 0);
const LAYOUT: Layout = Layout::new(*b"PLFNET01", VERSION, CRC_AT, Extent::Record);
const _: () = assert!(LAYOUT.fits(RECORD_BYTES));
const GENERATION_AT: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetStatus {
    Unopened,
    /// No record yet: the build's credentials, writable.
    Empty,
    /// A valid record (stored credentials, or a clear).
    Loaded,
    /// A newer schema version: read-only.
    ReadOnly,
    Io,
}

#[derive(Clone, Copy, Debug)]
pub struct NetStore<I> {
    units: DualSlot<I>,
    pub status: NetStatus,
    pub generation: u64,
    pub slot: c_uint,
    /// `None` means the build's credentials.
    pub credentials: Option<Credentials>,
}

impl<I> Default for NetStore<I> {
    fn default() -> Self {
        Self {
            units: DualSlot::unbound(&LAYOUT),
            status: NetStatus::Unopened,
            generation: 0,
            slot: 0,
            credentials: None,
        }
    }
}

fn encode(generation: u64, credentials: Option<&Credentials>) -> [u8; RECORD_BYTES] {
    let mut p = LAYOUT.record();
    put_u64::<GENERATION_AT, _>(&mut p, generation);
    if let Some(c) = credentials {
        // `Credentials::new` bounds both: 32 SSID bytes, 63 passphrase bytes.
        let (ssid, psk) = (c.ssid(), c.psk());
        p[20] = u8::try_from(ssid.len()).expect("an SSID is at most 32 bytes");
        p[21] = u8::try_from(psk.len()).expect("a passphrase is at most 63 bytes");
        p[22..22 + ssid.len()].copy_from_slice(ssid);
        p[54..54 + psk.len()].copy_from_slice(psk);
    }
    seal(&mut p);
    p
}

/// The generation and the credentials, `None` for a clear.
fn decode(p: &[u8; RECORD_BYTES]) -> Option<(u64, Option<Credentials>)> {
    let generation = get_u64::<GENERATION_AT, _>(p);
    let (ssid_len, psk_len) = (usize::from(p[20]), usize::from(p[21]));
    if generation == 0 || p[118..CRC_AT] != [0, 0] {
        return None;
    }
    if ssid_len == 0 {
        return (psk_len == 0 && p[22..118].iter().all(|&b| b == 0)).then_some((generation, None));
    }
    // Unused bytes must be zero, so one encoding exists per value.
    if ssid_len > MAX_SSID
        || psk_len > MAX_PSK
        || p[22 + ssid_len..54].iter().any(|&b| b != 0)
        || p[54 + psk_len..118].iter().any(|&b| b != 0)
    {
        return None;
    }
    Credentials::new(&p[22..22 + ssid_len], &p[54..54 + psk_len]).map(|c| (generation, Some(c)))
}

impl<I: SlotIo> NetStore<I> {
    /// Bind to two exclusive erase units and load the newest valid record.
    pub fn open(&mut self, io: I) -> NetStatus {
        *self = Self::default();
        self.status = match self.units.open_newest(io, decode) {
            Newest::Io => NetStatus::Io,
            Newest::Future => NetStatus::ReadOnly,
            Newest::Loaded {
                slot,
                generation,
                value,
            } => {
                self.slot = slot;
                self.generation = generation;
                self.credentials = value;
                NetStatus::Loaded
            }
            // The two sectors are this store's alone, so anything else there
            // is a save of ours cut short: no credentials, still writable.
            Newest::Nothing { .. } => NetStatus::Empty,
        };
        self.status
    }

    pub fn writable(&self) -> bool {
        matches!(self.status, NetStatus::Empty | NetStatus::Loaded)
    }

    /// Store `credentials`, or `None` to return to the build's, to the
    /// other slot by the torn-write rule; whether flash now holds them. A
    /// save that reports failure reads both units again,
    /// so memory names what flash holds and the store stays writable.
    pub fn save(&mut self, credentials: Option<&Credentials>) -> bool {
        if !self.writable() || !self.units.writable() {
            return false;
        }
        let saved = self
            .units
            .save_newest(self.generation, self.slot, |generation| {
                encode(generation, credentials)
            });
        let Some((slot, generation)) = saved else {
            return self.reread(self.generation + 1);
        };
        self.slot = slot;
        self.generation = generation;
        self.credentials = credentials.copied();
        true
    }

    /// After a save that reported failure: open the units again, so memory
    /// names what flash holds. The save may have landed whole before the
    /// failure was reported, or left its unit torn, which the newest-valid
    /// rule reads past and the next save writes over; either way the store
    /// stays writable. A re-read that fails too, or that reads less than the
    /// store held, keeps the value from before the save, marked `Io`: the
    /// device goes on with what it last knew rather than with nothing.
    /// Whether the save at `generation` landed.
    fn reread(&mut self, generation: u64) -> bool {
        let before = (self.slot, self.generation, self.credentials);
        let Some(io) = self.units.unbind() else {
            self.status = NetStatus::Io;
            return false;
        };

        // The save never touches the unit it replaces, so a store that held
        // a record reads that record or the new one again: anything older,
        // or nothing at all, is a misread. Taken for empty, it would let the
        // next save erase the unit still holding the current record.
        let held = before.1;
        match self.open(io) {
            NetStatus::Loaded if self.generation >= held => self.generation == generation,
            NetStatus::Empty if held == 0 => false,
            _ => {
                (self.slot, self.generation, self.credentials) = before;
                self.status = NetStatus::Io;
                false
            }
        }
    }
}
