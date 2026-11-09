//! The device settings as the runtime's services see them: the view the
//! settings writer publishes (the speaker honours `sound`, `volume` and the
//! sound window; the loop's lights honour `lights` and the lights window),
//! and a `settings` request's fields and result, carried by
//! `wellfed_hal::settings` between the device API and the writer.
//!
//! A device kind keeps settings of its own beside these (the feeder's meal
//! reminder and desiccant reminder, `wellfed_core_feeder::settings`), in a
//! record of its own: one request and one view carry both, the kind's part
//! as [`KindSettings`] names it, so nothing here names a kind.
//!
//! **One generation for both records.** A client checks a write against
//! the view's generation, so a generation must never name two different
//! settings. The view reports a generation only when both records read
//! their newest slot cleanly (the other valid or erased, nothing torn) at
//! the same generation ([`view_generation`]); a kind with no record of its
//! own counts the device's alone. Otherwise the view reports
//! [`NO_GENERATION`], read-only, and writes are refused.
//!
//! Records that do not agree come from a save that stopped partway (a
//! power cut, or a write that failed or reported failure after it landed)
//! or from a newest slot lost. The writer mends them once per boot, before
//! it takes a change: it saves both records again with what they hold, at
//! [`repair_generation`], two past the larger, a record that read cleanly
//! first. Until that lands, the view stays read-only for the boot, and the
//! lights and the speaker go by what the records hold. After a failed save
//! the writer reads both records again and publishes what they then say,
//! which is a shared generation only if both landed; a save that failed
//! records no history event. Every save of a change writes both records at
//! one past the shared generation, the device's first.
//!
//! Why two past the larger: a newest slot lost on each record hides the
//! generation both last held, one past what they read, and a repair at one
//! past would publish it again with other contents. Why the clean record
//! first: a write to a record that fell back erases its torn slot, the only
//! sign that it held more, so a repair that fails there could leave both
//! records reading clean at an old generation, and the next save would
//! repeat the one after it; a repair landing on the clean record first
//! keeps its generation in a valid slot.
//!
//! The `settings_model` test of `wellfed_device` checks this by brute
//! force: the stores' own code over a flash whose writes stop at every
//! step (an erase refused, cut short at either end, or finished and
//! reported failed, a body half programmed, a marker missing, a write that
//! landed and reported failure, each one the writer goes on from or a
//! power cut), boots whose first read of a record fails, the retry after a
//! failed save, and the newest or older valid slot of either record lost,
//! in every order. No generation is ever published for two different
//! settings, through 10 events (12.5 million states). Repairing the
//! device's record first fails at 8 (three slots lost and two repairs
//! failing), and repairing one past the larger at 5. Longer sequences are
//! unchecked.
//!
//! The cost is wear: a repair spends two generations of the wear limit's
//! count on each record, once per boot that finds the records apart, which
//! only a cut or failed save or a lost slot causes, and once more after a
//! failed save's pause (`REPAIR_RETRY_MS` in the writer). A record whose
//! saves keep failing makes every boot rewrite the other at two past, two
//! of its 100 000 generations each time, until it reads `Worn` and refuses
//! saves.

/// Minutes in a local day: a window's bounds are below it.
pub const DAY_MINUTES: u16 = 24 * 60;

/// When a light or sound setting that is on may show or play, as stock's
/// "aging type" selects: always, or only inside a window of the local day.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Window {
    #[default]
    Always,
    /// From minute `start` to minute `end` of the local day, both included;
    /// a window whose end is before its start wraps past midnight, and
    /// `start == end` is that one minute, as stock compares them.
    Between { start: u16, end: u16 },
}

impl Window {
    /// Whether both bounds are minutes of a day.
    pub const fn valid(self) -> bool {
        match self {
            Self::Always => true,
            Self::Between { start, end } => start < DAY_MINUTES && end < DAY_MINUTES,
        }
    }

    /// Whether the window allows the local wall time `local_ms` (Unix
    /// milliseconds as if local were UTC, `None` while the clock is
    /// invalid or has no offset). Without a local time every window
    /// allows: lights and sound behave as always on until the clock syncs,
    /// so a fault light is never hidden by a clock the feeder lost.
    pub fn allows(self, local_ms: Option<i64>) -> bool {
        let Self::Between { start, end } = self else {
            return true;
        };
        let Some(local_ms) = local_ms else {
            return true;
        };

        let minute = local_ms
            .div_euclid(60_000)
            .rem_euclid(i64::from(DAY_MINUTES));
        let (start, end) = (i64::from(start), i64::from(end));
        if start <= end {
            (start..=end).contains(&minute)
        } else {
            minute >= start || minute <= end
        }
    }
}

/// A device kind's own settings, carried beside the device's in one
/// `settings` request and one view. `()` is a kind with none.
pub trait KindSettings: Copy + Default + PartialEq + Eq + core::fmt::Debug {
    /// A request's fields for the kind's settings; none present changes
    /// nothing.
    type Fields: Copy + Default + PartialEq + Eq + core::fmt::Debug;

    /// The settings before any record is read, and the fields of a request
    /// that changes nothing: the constants a mailbox starts from.
    const DEFAULT: Self;
    const NO_FIELDS: Self::Fields;

    /// Whether `fields` change anything (and so must be signed).
    fn changes(fields: &Self::Fields) -> bool;
    /// Whether every field present is one the kind's store accepts.
    fn valid(fields: &Self::Fields) -> bool;
    /// These settings with `fields` applied.
    #[must_use]
    fn changed(self, fields: &Self::Fields) -> Self;

    /// Whether the kind keeps a record of its own, saved beside the
    /// device's at the same generation; false for `()`.
    const OWN_RECORD: bool;
}

/// How a settings record read: its generation, and whether it read
/// cleanly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordRead {
    pub generation: u64,
    pub slots: Slots,
}

/// Whether a record read its newest slot with the other in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slots {
    /// Both slots valid, or the other erased; or both erased.
    Clean,
    /// Anything else: the other slot torn or invalid (a newest slot lost,
    /// or a write cut short), no valid record, a read or write that failed,
    /// or a record not yet opened.
    Unclean,
}

/// The generation a view reports while the records do not share one, read
/// only. No record ever holds it, since saves stop at the wear limit far
/// below, so a client that sends it back as a write's base is refused.
pub const NO_GENERATION: u32 = u32::MAX;

/// The generation both records share, from the device's record and, for a
/// kind with a record of its own, the kind's: `None` unless each read
/// cleanly and they agree (the module's docs give the rule).
pub const fn view_generation(device: RecordRead, kind: Option<RecordRead>) -> Option<u64> {
    if !matches!(device.slots, Slots::Clean) {
        return None;
    }
    match kind {
        None => Some(device.generation),
        Some(kind)
            if matches!(kind.slots, Slots::Clean) && kind.generation == device.generation =>
        {
            Some(device.generation)
        }
        Some(_) => None,
    }
}

/// The generation a boot's repair saves both records at: two past the
/// larger of their generations (the module's docs say why two).
pub const fn repair_generation(device: u64, kind: u64) -> u64 {
    let larger = if kind > device { kind } else { device };
    larger.saturating_add(2)
}

/// How the writer finished the settings changes it took this boot, for a
/// client's write waiting on its own: how many it finished, and which of
/// the last 32 failed. A change finishes as the next number, since only
/// one waits at a time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Saved {
    /// Changes finished this boot.
    pub finished: u32,
    /// Bit `n` set when the change finished `n` before the latest failed:
    /// nothing saved, or only part of it.
    pub failed: u32,
}

/// What became of one settings change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    /// Not finished yet.
    Pending,
    /// Saved whole.
    Done,
    /// Not saved, or only part of it; also a change finished so long ago
    /// that its bit is gone, which a client never waits that long for.
    Failed,
}

impl Saved {
    /// One more change finished.
    pub fn finish(&mut self, failed: bool) {
        self.finished = self.finished.wrapping_add(1);
        self.failed = (self.failed << 1) | u32::from(failed);
    }

    /// The number the next change queued will finish as.
    pub fn next(&self) -> u32 {
        self.finished.wrapping_add(1)
    }

    /// What became of the change that finished as `change`.
    pub fn outcome(&self, change: u32) -> SaveOutcome {
        let age = self.finished.wrapping_sub(change);
        if age > u32::MAX / 2 {
            SaveOutcome::Pending
        } else if age >= u32::BITS || self.failed & (1 << age) != 0 {
            SaveOutcome::Failed
        } else {
            SaveOutcome::Done
        }
    }
}

impl KindSettings for () {
    type Fields = ();

    const DEFAULT: Self = ();
    const NO_FIELDS: Self::Fields = ();

    fn changes(_: &()) -> bool {
        false
    }

    fn valid(_: &()) -> bool {
        true
    }

    fn changed(self, _: &()) -> Self {}

    const OWN_RECORD: bool = false;
}

/// The fields of a `settings` request; none present reads the settings.
/// `kind` is the device kind's own ([`KindSettings::Fields`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SettingsFields<K: KindSettings = ()> {
    pub lights: Option<bool>,
    pub sound: Option<bool>,
    pub volume: Option<u8>,
    pub lights_window: Option<Window>,
    pub sound_window: Option<Window>,
    pub kind: K::Fields,
}

impl<K: KindSettings> SettingsFields<K> {
    /// A request that changes nothing: what a read sends.
    pub const NONE: Self = Self {
        lights: None,
        sound: None,
        volume: None,
        lights_window: None,
        sound_window: None,
        kind: K::NO_FIELDS,
    };

    /// Whether the request changes anything (and so must be signed).
    pub fn changes(&self) -> bool {
        self.lights.is_some()
            || self.sound.is_some()
            || self.volume.is_some()
            || self.lights_window.is_some()
            || self.sound_window.is_some()
            || K::changes(&self.kind)
    }

    /// Whether every field present is one the stores accept.
    pub fn valid(&self) -> bool {
        self.volume.is_none_or(|volume| volume <= MAX_VOLUME)
            && self.lights_window.is_none_or(Window::valid)
            && self.sound_window.is_none_or(Window::valid)
            && K::valid(&self.kind)
    }
}

/// Largest `volume` a `settings` request may set, and the store accepts.
/// Volume 0 mutes every sound, as stock's does.
pub const MAX_VOLUME: u8 = 100;

/// The settings as a `settings` reply reports them. `kind` is the device
/// kind's own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SettingsView<K = ()> {
    pub lights: bool,
    pub sound: bool,
    pub volume: u8,
    pub lights_window: Window,
    pub sound_window: Window,
    pub kind: K,
    /// The records' shared generation ([`view_generation`]): every save
    /// raises it, so a write checked against it is refused after any
    /// change; 0 for defaults never saved, and [`NO_GENERATION`] while the
    /// records do not share one.
    pub generation: u32,
    /// Whether NOR holds records this firmware can update: both, since one
    /// request may change either, and sharing a generation.
    pub writable: bool,
    /// How the writer finished the changes it took this boot.
    pub saved: Saved,
}

impl<K> SettingsView<K> {
    /// The device's settings alone, as the SDK's own services read them.
    pub fn device(&self) -> SettingsView {
        SettingsView {
            lights: self.lights,
            sound: self.sound,
            volume: self.volume,
            lights_window: self.lights_window,
            sound_window: self.sound_window,
            kind: (),
            generation: self.generation,
            writable: self.writable,
            saved: self.saved,
        }
    }

    /// Whether the lights may show at `local_ms` ([`Window::allows`]).
    pub fn lights_at(&self, local_ms: Option<i64>) -> bool {
        self.lights && self.lights_window.allows(local_ms)
    }

    /// Whether a sound may play at `local_ms`: the switch on, a volume
    /// above 0 and inside the sound window.
    pub fn sound_at(&self, local_ms: Option<i64>) -> bool {
        self.sound && self.volume > 0 && self.sound_window.allows(local_ms)
    }
}

/// How a `settings` request ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsResult<K = ()> {
    /// The current settings; `pending` once a change is queued for the writer.
    Current {
        view: SettingsView<K>,
        pending: bool,
    },
    /// Not loaded yet, or a change already pending.
    Busy,
    /// The store cannot be written (corrupt, newer schema, worn or unarmed).
    ReadOnly,
}

/// `always`, or the window's bounds as `HH:MM-HH:MM`, as records spell it.
/// `wellfed_model::Window` (the CLI's and JSON clients' type, which this
/// no_std crate cannot share) spells it the same, and parses it.
impl core::fmt::Display for Window {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::Always => f.write_str("always"),
            Self::Between { start, end } => write!(
                f,
                "{:02}:{:02}-{:02}:{:02}",
                start / 60,
                start % 60,
                end / 60,
                end % 60
            ),
        }
    }
}
