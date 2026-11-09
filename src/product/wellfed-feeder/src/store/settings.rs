//! The feeder's own settings record (sectors 48 and 49), beside the
//! device's (`wellfed_device::store::settings`, sectors 2 and 3): the
//! meal reminder, the desiccant reminder and the bowl mode, which only a
//! feeder keeps. The same store holds both records, each with its own
//! fields ([`FeederRecord`]), so their torn-write rule, recovery and wear
//! limit are one; the settings writer saves this one when a request
//! changes the feeder's part.
//!
//! The record, major 1, magic `PLFFDS01`, 128 bytes: the slot header, the
//! generation (u64 at 12) and the binding (u64 at 20), then the bowl mode
//! at 28, the meal reminder's lead in minutes at 29 (0 off), the desiccant
//! reminder's period in days at 30, 1 at 31 when a replacement is
//! recorded, the replacement's local calendar day as an i32 at 32..36 (zero
//! when none), spare room from 36, CRC-32/ISO-HDLC of bytes 0..120 at 120
//! and the `CMIT` commit marker at 124.
//!
//! Bank 0 reads this record as it is written now for the life of the
//! feeder, and the store reads its units whole, so the record keeps one
//! length: a later field is a minor in the 84 spare bytes, which bank 0
//! carries unread, and a change that is not additive is a new store in
//! new sectors. A factory reset erases the record (it is off the reset's
//! keep list) and the handover from stock keeps it, as it does the
//! device's.

use wellfed_core::store::slot::{Extent, Layout, RECORD_BYTES, Version, get_u32, put_u32};
use wellfed_core_feeder::feed::progress::BowlMode;
use wellfed_core_feeder::settings::{FeederSettings, MealReminder};
use wellfed_device::store::settings::{SettingsRecord, SettingsStore};

pub const VERSION: Version = Version::new(1, 0);
const LAYOUT: Layout = Layout::new(*b"PLFFDS01", VERSION, FIELDS_END, Extent::Unit);
const _: () = assert!(LAYOUT.fits(RECORD_BYTES));
/// Where the spare room starts.
const FIELDS_END: usize = 36;
const _: () = assert!(RECORD_BYTES - 8 - FIELDS_END == 84);

/// The feeder's settings in their two units.
pub type FeederSettingsStore<I> = SettingsStore<I, FeederRecord>;

/// The feeder's settings record's fields.
#[derive(Clone, Copy, Debug)]
pub enum FeederRecord {}

impl SettingsRecord for FeederRecord {
    type Value = FeederSettings;

    const LAYOUT: &'static Layout = &LAYOUT;

    fn encode(s: &FeederSettings, p: &mut [u8; RECORD_BYTES]) {
        p[28] = s.bowl_mode.byte();
        p[29] = s.meal_reminder.minutes().unwrap_or(0);
        p[30] = s.desiccant_days;
        p[31] = u8::from(s.desiccant_reset.is_some());
        put_u32::<32, _>(p, s.desiccant_reset.unwrap_or(0).cast_unsigned());
    }

    fn decode(p: &[u8; RECORD_BYTES]) -> Option<FeederSettings> {
        Some(FeederSettings {
            bowl_mode: BowlMode::from_byte(p[28])?,
            meal_reminder: match p[29] {
                0 => MealReminder::Off,
                minutes => MealReminder::Before { minutes },
            },
            desiccant_days: p[30],
            desiccant_reset: match p[31] {
                0 if get_u32::<32, _>(p) == 0 => None,
                1 => Some(get_u32::<32, _>(p).cast_signed()),
                _ => return None,
            },
        })
    }

    fn valid(s: &FeederSettings) -> bool {
        s.valid()
    }
}
