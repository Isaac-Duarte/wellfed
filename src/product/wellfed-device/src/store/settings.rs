//! Two-slot alternating settings records, in the dual-slot store
//! (`wellfed_core::store::slot`) as the feed journal is: the device's own
//! ([`DeviceRecord`], sectors 2 and 3), and beside it a device kind's
//! (the feeder's is `wellfed_feeder::store::settings`, sectors 48 and 49),
//! each a [`SettingsStore`] over its own [`SettingsRecord`]. One `settings`
//! request and one view carry both ([`Settings::view`]), and every save
//! writes both at one shared generation, the device's first
//! ([`save_both`]); a boot that finds them apart saves both again first
//! ([`repair`]). The rule, and why it never names two different settings,
//! are in `wellfed_core::settings`.
//!
//! Every record is 128 bytes: the slot header (magic, major, minor, record
//! size), the generation (u64 at 12), the binding (u64 at 20), the record's
//! own fields from 28, zero spare room, CRC-32/ISO-HDLC of bytes 0..120 at
//! 120 and the `CMIT` commit marker at 124.
//!
//! The device's record, major 5, magic `PLFSET01`: flags at 28 (bit 0
//! lights, bit 1 sound), volume at 29, the lights and sound windows (a kind
//! byte each at 30 and 31, 0 always and 1 between, then each window's start
//! and end minute as u16 at 32..36 and 36..40), and spare room from 40.
//! Major 4 held the feeder's fields too (bowl mode, meal reminder,
//! desiccant reminder), which moved to the feeder's record, and an unused
//! UTC offset, which the time settings' zone replaced; a major-4 record is
//! superseded and counts as erased.
//!
//! **Room to grow.** Bank 0 keeps the build it was written with for the
//! life of the feeder, so it reads these records as they are written now:
//! a later build may only add fields as a minor, in the spare room, which
//! bank 0 carries unread through its own saves. The store reads its units
//! whole, so its length never changes within a major, and a change that is
//! not additive is a new store in new sectors. So each record keeps its
//! fields short and leaves the rest of the 120 bytes spare: 80 bytes in the
//! device's, 84 in the feeder's.
//!
//! Unlike the journal, losing the newest settings change is safe, so a torn
//! or invalid newer slot falls back to the older valid one. Storage with no
//! valid record, or holding a newer major, reads as defaults and is
//! never written until an explicit [`SettingsStore::reset`]: this firmware
//! does not overwrite data it cannot interpret. Only a save of its own cut
//! short beside an erased unit ([`SlotState::Dirty`]), or a record of an
//! older major (settings are defaults-safe, so this is the one store that
//! starts over on one), still reads as empty.

use core::ffi::c_uint;

use wellfed_core::settings::{
    KindSettings, NO_GENERATION, RecordRead, Saved, SettingsFields, SettingsView, Slots, Window,
    repair_generation, view_generation,
};
use wellfed_core::store::slot::{
    DualSlot, Extent, Found, Layout, Pick, RECORD_BYTES, SlotIo, Version, get_u16, get_u64, newest,
    put_u16, put_u64, seal, target,
};

/// The device record's version.
pub const VERSION: Version = Version::new(5, 0);
/// Settings read their whole units, as the journal does, so data another
/// writer left is never taken for empty settings and overwritten.
const DEVICE_LAYOUT: Layout = Layout::new(*b"PLFSET01", VERSION, 40, Extent::Unit);
const _: () = assert!(DEVICE_LAYOUT.fits(RECORD_BYTES));
const GENERATION_AT: usize = 12;
const BINDING_AT: usize = 20;
/// Where a record's own fields start.
pub const FIELDS_AT: usize = 28;

/// Generations written before saves are refused: about 50 000 erases per
/// unit, half a typical NOR part's 100 000-cycle rating.
pub const WEAR_LIMIT: u64 = 100_000;

pub use wellfed_core::settings::MAX_VOLUME;

/// One settings record's own fields: its layout, and its values to and
/// from the bytes from [`FIELDS_AT`]. The store owns the header, the
/// generation, the binding, the checksum and the marker.
pub trait SettingsRecord {
    type Value: Copy + Default + PartialEq + Eq + core::fmt::Debug;

    /// The record's magic, version and where its fields end.
    const LAYOUT: &'static Layout;

    /// Write `value`'s fields; every other byte is zero already.
    fn encode(value: &Self::Value, p: &mut [u8; RECORD_BYTES]);
    /// The value a record holds, or `None` when a field is not one this
    /// build writes (an unknown byte, or bits or padding set).
    fn decode(p: &[u8; RECORD_BYTES]) -> Option<Self::Value>;
    /// Whether the store accepts `value`.
    fn valid(value: &Self::Value) -> bool;
}

/// The device's settings, those every kind keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub lights: bool,
    pub sound: bool,
    pub volume: u8,
    pub lights_window: Window,
    pub sound_window: Window,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lights: true,
            sound: true,
            volume: 50,
            lights_window: Window::Always,
            sound_window: Window::Always,
        }
    }
}

impl Settings {
    pub fn valid(&self) -> bool {
        self.volume <= MAX_VOLUME && self.lights_window.valid() && self.sound_window.valid()
    }

    /// These settings with a request's device fields applied; the writer's
    /// merge, shared with the host simulator so the two cannot drift.
    pub fn changed<K: KindSettings>(self, fields: &SettingsFields<K>) -> Self {
        Self {
            lights: fields.lights.unwrap_or(self.lights),
            sound: fields.sound.unwrap_or(self.sound),
            volume: fields.volume.unwrap_or(self.volume),
            lights_window: fields.lights_window.unwrap_or(self.lights_window),
            sound_window: fields.sound_window.unwrap_or(self.sound_window),
        }
    }

    /// The view the writer publishes for these settings, as the device's
    /// record read (`device`), with the kind's beside them: the shared
    /// generation the two records name ([`shared_generation`]), and
    /// writable only when both are and share one; while they do not, the
    /// view reports [`NO_GENERATION`], read-only, with what each record
    /// holds, which the lights and the speaker still go by. The outcomes of
    /// finished changes are the mailbox's, which keeps its own.
    ///
    /// One read-only record makes the whole view read-only, refusing even a
    /// change to the other record's fields. That is deliberate: a record
    /// opens read-only only when it is a newer major or corrupt, and no
    /// client offers a feeder an older build, so neither happens in use, and
    /// a single writable flag keeps one request's answer whole.
    pub fn view<K: KindSettings>(
        &self,
        device: RecordRead,
        writable: bool,
        kind: Stored<K>,
    ) -> SettingsView<K> {
        // Saves stop at the wear limit, far below 32 bits.
        let generation = shared_generation(device, &kind).and_then(|g| u32::try_from(g).ok());
        SettingsView {
            lights: self.lights,
            sound: self.sound,
            volume: self.volume,
            lights_window: self.lights_window,
            sound_window: self.sound_window,
            kind: kind.settings,
            generation: generation.unwrap_or(NO_GENERATION),
            writable: writable && kind.writable && generation.is_some(),
            saved: Saved::default(),
        }
    }
}

/// The generation both records share, the device's as it read and the
/// kind's (none for a kind without a record): what the view reports, and
/// one below what the next save writes to both; `None` while they do not
/// share one (`wellfed_core::settings::view_generation`).
pub fn shared_generation<K: KindSettings>(device: RecordRead, kind: &Stored<K>) -> Option<u64> {
    view_generation(device, K::OWN_RECORD.then(|| kind.read()))
}

/// How a store's status says its record read: clean only when it opened
/// [`SettingsStatus::Loaded`] or [`SettingsStatus::Empty`], or its last
/// save landed. A record that fell back from a torn slot, holds nothing
/// valid, failed a read or a write, or is worn or unopened is not: its
/// generation names nothing a client may write against.
const fn slots(status: SettingsStatus) -> Slots {
    match status {
        SettingsStatus::Loaded | SettingsStatus::Empty => Slots::Clean,
        _ => Slots::Unclean,
    }
}

/// Which of the two records a save stopped at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Device,
    /// The device kind's own record.
    Kind,
}

/// What a boot's [`repair`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repair {
    /// The records already shared a generation: nothing written.
    Agreed,
    /// Both saved again at this generation, which they now share.
    Repaired(u64),
    /// A record may not be written (no valid record, a newer major, a
    /// failed read, or flash this boot may not write): nothing written,
    /// and the view stays read-only for the boot.
    Refused,
    /// The save of `part` failed, with `status`; both were read again.
    Failed { part: Part, status: SettingsStatus },
}

/// What [`save_both`] did with a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveBoth {
    /// Both saved at this generation, which they now share.
    Saved(u64),
    /// The records share no generation: nothing written.
    Apart,
    /// The save of `part` failed, with `status`; both were read again.
    Failed { part: Part, status: SettingsStatus },
}

/// The boot's repair: when the records do not share a generation, save
/// both again with what each holds at
/// [`wellfed_core::settings::repair_generation`], two past the larger, the
/// device's first; the caller holds the write turn and the attempt marker.
/// A failed save reads both again, as [`save_both`] does.
pub fn repair<I: SlotIo + Clone, K: KindStore>(
    device: &mut SettingsStore<I>,
    kind: &mut K,
) -> Repair {
    let stored = kind.stored();
    if shared_generation(device.read(), &stored).is_some() {
        return Repair::Agreed;
    }
    if !device.writable() || !stored.writable {
        return Repair::Refused;
    }

    let generation = repair_generation(device.generation, stored.generation);
    let settings = device.settings;
    let first = if matches!(device.read().slots, Slots::Clean) {
        Part::Device
    } else {
        Part::Kind
    };
    match write_both(device, kind, &settings, &stored.settings, generation, first) {
        Ok(()) => Repair::Repaired(generation),
        Err((part, status)) => Repair::Failed { part, status },
    }
}

/// Save a change to both records at one past the generation they share,
/// the device's first; the caller holds the write turn and the attempt
/// marker. Nothing is written while they share none. When either save
/// fails (which may mean it landed, a write being able to complete and
/// still report failure), both records are read again from flash, so the
/// view names only what they hold: a generation they share, or none.
pub fn save_both<I: SlotIo + Clone, K: KindStore>(
    device: &mut SettingsStore<I>,
    kind: &mut K,
    settings: &Settings,
    kind_settings: &K::Settings,
) -> SaveBoth {
    let Some(shared) = shared_generation(device.read(), &kind.stored()) else {
        return SaveBoth::Apart;
    };

    let generation = shared + 1;
    match write_both(
        device,
        kind,
        settings,
        kind_settings,
        generation,
        Part::Device,
    ) {
        Ok(()) => SaveBoth::Saved(generation),
        Err((part, status)) => SaveBoth::Failed { part, status },
    }
}

/// Save both records at `generation`, `first` first; on a failure, read
/// both again and name the record that failed, with its status before the
/// reading.
fn write_both<I: SlotIo + Clone, K: KindStore>(
    device: &mut SettingsStore<I>,
    kind: &mut K,
    settings: &Settings,
    kind_settings: &K::Settings,
    generation: u64,
    first: Part,
) -> Result<(), (Part, SettingsStatus)> {
    let mut save = |part| match part {
        Part::Device => device
            .save_at(settings, generation)
            .then_some(())
            .ok_or((part, device.status)),
        Part::Kind if K::Settings::OWN_RECORD => kind
            .save(kind_settings, generation)
            .then_some(())
            .ok_or_else(|| (part, kind.stored().status)),
        Part::Kind => Ok(()),
    };
    let second = match first {
        Part::Device => Part::Kind,
        Part::Kind => Part::Device,
    };
    let failed = save(first).and_then(|()| save(second)).err();

    let Some(failed) = failed else {
        return Ok(());
    };
    device.reopen();
    kind.reopen();
    Err(failed)
}

/// The device's record (sectors 2 and 3).
#[derive(Clone, Copy, Debug)]
pub enum DeviceRecord {}

impl SettingsRecord for DeviceRecord {
    type Value = Settings;

    const LAYOUT: &'static Layout = &DEVICE_LAYOUT;

    fn encode(s: &Settings, p: &mut [u8; RECORD_BYTES]) {
        p[28] = u8::from(s.lights) | (u8::from(s.sound) << 1);
        p[29] = s.volume;
        let (kind, start, end) = window_fields(s.lights_window);
        p[30] = kind;
        put_u16::<32, _>(p, start);
        put_u16::<34, _>(p, end);
        let (kind, start, end) = window_fields(s.sound_window);
        p[31] = kind;
        put_u16::<36, _>(p, start);
        put_u16::<38, _>(p, end);
    }

    fn decode(p: &[u8; RECORD_BYTES]) -> Option<Settings> {
        if p[28] & !3 != 0 {
            return None;
        }
        Some(Settings {
            lights: p[28] & 1 != 0,
            sound: p[28] & 2 != 0,
            volume: p[29],
            lights_window: window(p[30], get_u16::<32, _>(p), get_u16::<34, _>(p))?,
            sound_window: window(p[31], get_u16::<36, _>(p), get_u16::<38, _>(p))?,
        })
    }

    fn valid(s: &Settings) -> bool {
        s.valid()
    }
}

/// What a settings store holds, as the writer publishes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stored<V> {
    pub settings: V,
    /// The record's generation; 0 for defaults never saved.
    pub generation: u64,
    /// Whether NOR holds a record this firmware can update.
    pub writable: bool,
    /// How the store opened, or its last save's failure, as the writer's
    /// records report it.
    pub status: SettingsStatus,
}

impl<V> Stored<V> {
    /// How the record read, for the shared generation.
    pub const fn read(&self) -> RecordRead {
        RecordRead {
            generation: self.generation,
            slots: slots(self.status),
        }
    }
}

/// A device kind's own settings record, kept beside the device's: the
/// settings writer and the simulator save the kind's part of a request
/// through it. `()` is a kind with none, which holds nothing and saves
/// nothing.
pub trait KindStore {
    type Settings: KindSettings;

    /// What it holds.
    fn stored(&self) -> Stored<Self::Settings>;
    /// Save `settings` at `generation`, the one the device's record was
    /// just saved at; false when the save failed or was refused.
    fn save(&mut self, settings: &Self::Settings, generation: u64) -> bool;
    /// Read the record again from flash, after a save failed.
    fn reopen(&mut self);
}

impl KindStore for () {
    type Settings = ();

    fn stored(&self) -> Stored<()> {
        Stored {
            settings: (),
            generation: 0,
            writable: true,
            status: SettingsStatus::Loaded,
        }
    }

    fn save(&mut self, _: &(), _: u64) -> bool {
        true
    }

    fn reopen(&mut self) {}
}

impl<I: SlotIo + Clone, R: SettingsRecord> KindStore for SettingsStore<I, R>
where
    R::Value: KindSettings,
{
    type Settings = R::Value;

    fn stored(&self) -> Stored<R::Value> {
        SettingsStore::stored(self)
    }

    fn save(&mut self, settings: &R::Value, generation: u64) -> bool {
        self.save_at(settings, generation)
    }

    fn reopen(&mut self) {
        SettingsStore::reopen(self);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum SettingsStatus {
    /// A valid record; the other slot is erased or holds the older one.
    #[default]
    Loaded = 0,
    /// Each slot erased or holding an older format version: defaults,
    /// writable.
    Empty = 1,
    /// A valid record, with the other slot torn or invalid.
    Recovered = 2,
    /// No valid record: defaults, read-only until reset.
    Corrupt = 3,
    /// A newer schema version is present: defaults, read-only until reset.
    Future = 4,
    Io = 5,
    /// The wear limit is reached: saves are refused.
    Worn = 6,
    Unopened = 7,
}

impl SettingsStatus {
    /// The variant's number, as records and reports print it.
    pub const fn code(self) -> u32 {
        self as u32
    }

    /// Whether a store opened with this status may be written.
    pub const fn writable(self) -> bool {
        matches!(self, Self::Loaded | Self::Empty | Self::Recovered)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum SlotState {
    /// Not read by the last open.
    #[default]
    Unread = 0,
    Erased = 1,
    Valid = 2,
    Future = 3,
    /// A prefix of our own record without its commit: a save cut short.
    Dirty = 4,
    /// Anything else, including a unit whose erase was cut short.
    Invalid = 5,
    /// A record in an older format version of this store, superseded: no
    /// build migrates it, so it counts as erased.
    Older = 6,
}

impl SlotState {
    /// The variant's number, as records and reports print it.
    pub const fn code(self) -> u32 {
        self as u32
    }

    /// The torn-write rule's state as settings report it: only a save of
    /// its own cut short is dirty.
    fn of<V>(found: &Found<V>) -> Self {
        match found {
            Found::Erased => Self::Erased,
            Found::Valid(_) => Self::Valid,
            Found::Future => Self::Future,
            Found::Older => Self::Older,
            Found::Partial => Self::Dirty,
            Found::Unmarked | Found::Torn | Found::Invalid => Self::Invalid,
        }
    }
}

/// A settings record's two units: the device's by default, a kind's
/// with its own [`SettingsRecord`].
#[derive(Clone, Copy, Debug)]
pub struct SettingsStore<I, R: SettingsRecord = DeviceRecord> {
    units: DualSlot<I>,
    binding: u64,
    pub settings: R::Value,
    pub status: SettingsStatus,
    pub generation: u64,
    pub slot: c_uint,
    /// What the last open found in each slot.
    pub slots: [SlotState; 2],
    writable: bool,
}

impl<I, R: SettingsRecord> Default for SettingsStore<I, R> {
    fn default() -> Self {
        Self {
            units: DualSlot::unbound(R::LAYOUT),
            binding: 0,
            settings: R::Value::default(),
            status: SettingsStatus::Unopened,
            generation: 0,
            slot: 0,
            slots: [SlotState::Unread; 2],
            writable: false,
        }
    }
}

impl<I, R: SettingsRecord> SettingsStore<I, R> {
    /// Whether a save may be written: opened writable, and not worn or
    /// failed since.
    pub fn writable(&self) -> bool {
        self.writable && self.status.writable()
    }

    /// How its record read, for the shared generation.
    pub const fn read(&self) -> RecordRead {
        RecordRead {
            generation: self.generation,
            slots: slots(self.status),
        }
    }

    /// What it holds, as the writer publishes it.
    pub fn stored(&self) -> Stored<R::Value> {
        Stored {
            settings: self.settings,
            generation: self.generation,
            writable: self.writable(),
            status: self.status,
        }
    }
}

impl<I: SlotIo, R: SettingsRecord> SettingsStore<I, R> {
    fn encode(&self, generation: u64, value: &R::Value) -> [u8; RECORD_BYTES] {
        let mut p = R::LAYOUT.record();
        put_u64::<GENERATION_AT, _>(&mut p, generation);
        put_u64::<BINDING_AT, _>(&mut p, self.binding);
        R::encode(value, &mut p);
        seal(&mut p);
        p
    }

    /// The generation and the value of a record whose header and checksum
    /// hold.
    fn decode(&self, p: &[u8; RECORD_BYTES]) -> Option<(u64, R::Value)> {
        if get_u64::<BINDING_AT, _>(p) != self.binding {
            return None;
        }
        let generation = get_u64::<GENERATION_AT, _>(p);
        let value = R::decode(p)?;
        if !R::valid(&value) || generation == 0 {
            return None;
        }
        Some((generation, value))
    }

    fn scan(&self, slot: c_uint) -> Option<Found<(u64, R::Value)>> {
        self.units.scan(slot, |_, p| self.decode(p))
    }

    /// Bind to two exclusive erase units and load the newest valid record.
    pub fn open(&mut self, io: I, binding: u64) -> SettingsStatus {
        let unit = io.unit_bytes();
        *self = Self {
            units: DualSlot::new(io, R::LAYOUT),
            binding,
            ..Self::default()
        };

        if (unit as usize) < RECORD_BYTES || !unit.is_multiple_of(4) || binding == 0 {
            return self.status;
        }

        let (Some(a), Some(b)) = (self.scan(0), self.scan(1)) else {
            self.status = SettingsStatus::Io;
            return self.status;
        };
        self.slots = [SlotState::of(&a), SlotState::of(&b)];

        // A record in an older version of this store is superseded, never
        // migrated: it counts as an erased unit, so a format change starts
        // from writable defaults rather than locking the store.
        let blank = |found: &Found<_>| found.is_erased() || matches!(found, Found::Older);
        let both_valid = a.is_valid() && b.is_valid();
        let either_erased = blank(&a) || blank(&b);
        // A torn first save never committed anything: still empty.
        let first_cut = (matches!(a, Found::Partial) && blank(&b))
            || (blank(&a) && matches!(b, Found::Partial));
        self.status = match newest(a, b, |&(generation, _)| generation) {
            // A newer schema anywhere means this firmware must not write.
            Pick::Future => SettingsStatus::Future,
            Pick::Newest(slot, (generation, settings)) => {
                self.slot = slot;
                self.generation = generation;
                self.settings = settings;
                if both_valid || either_erased {
                    SettingsStatus::Loaded
                } else {
                    SettingsStatus::Recovered
                }
            }
            Pick::Nothing if (blank(&a) && blank(&b)) || first_cut => SettingsStatus::Empty,
            Pick::Nothing => SettingsStatus::Corrupt,
        };

        self.writable = self.status.writable();

        self.status
    }

    /// Open again on the I/O and binding it was opened with: read the
    /// record from flash as a boot would, after a save failed.
    pub fn reopen(&mut self) -> SettingsStatus
    where
        I: Clone,
    {
        let Some(io) = self.units.io().cloned() else {
            return self.status;
        };
        self.open(io, self.binding)
    }

    /// Write `settings` to the other slot by the torn-write rule. A failure
    /// keeps the previous settings in memory; the slot it touched may be
    /// torn, which the next open falls back from.
    pub fn save(&mut self, settings: &R::Value) -> bool {
        self.save_at(settings, self.generation + 1)
    }

    /// [`Self::save`] at `generation`, which must be past the record's own:
    /// the settings' shared generation, which [`save_both`] and [`repair`]
    /// give each record.
    pub fn save_at(&mut self, settings: &R::Value, generation: u64) -> bool {
        if !self.writable || !R::valid(settings) || generation <= self.generation {
            return false;
        }

        if generation > WEAR_LIMIT {
            self.status = SettingsStatus::Worn;
            return false;
        }

        let slot = target(self.generation, self.slot);
        let from = (self.generation > 0).then_some(self.slot);
        if !self
            .units
            .replace(slot, from, &self.encode(generation, settings))
        {
            self.status = SettingsStatus::Io;
            return false;
        }

        self.slot = slot;
        self.generation = generation;
        self.settings = *settings;
        self.status = SettingsStatus::Loaded;
        true
    }

    /// Explicit reset (owner action): erase both units and return to writable
    /// defaults. Clears CORRUPT and FUTURE. Generation restarts at 0, so wear
    /// accounting must not depend on it across resets.
    pub fn reset(&mut self) -> bool {
        if self.binding == 0 {
            return false;
        }

        let ok = self.units.clear();

        self.settings = R::Value::default();
        self.generation = 0;
        self.slot = 0;
        self.writable = ok;
        self.status = if ok {
            SettingsStatus::Empty
        } else {
            SettingsStatus::Io
        };
        ok
    }
}

/// A window's kind byte and its bounds, as the record keeps them (zero for
/// always).
fn window_fields(window: Window) -> (u8, u16, u16) {
    match window {
        Window::Always => (0, 0, 0),
        Window::Between { start, end } => (1, start, end),
    }
}

/// The window a record's kind byte and bounds name; `None` for an unknown
/// kind, or an always window with bounds.
fn window(kind: u8, start: u16, end: u16) -> Option<Window> {
    match kind {
        0 if start == 0 && end == 0 => Some(Window::Always),
        1 => Some(Window::Between { start, end }),
        _ => None,
    }
}
