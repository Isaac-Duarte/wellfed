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
//! settings. Every save writes both records with the same new generation,
//! one past the view's, the device's record first, and the view reports
//! ([`view_generation`]):
//!
//! - the records' generation, when they agree and each read its newest
//!   slot cleanly;
//! - one past the larger, when they read cleanly but differ: a save that
//!   stopped between the two writes (a power cut, or the second write
//!   failing);
//! - four past the larger, when either record fell back to its older slot
//!   (its other slot unreadable: a newest slot lost, or a write cut short).
//!
//! The next save, one past that, makes them agree again. The steps stay
//! small because a record's wear limit counts its generation.
//!
//! Why no generation names two different settings, with power cuts and
//! failed writes at any point and at most one record without its newest
//! slot at a time. Let S be the generation the last save wrote to the
//! device's record. First, every save writes past everything published
//! before it, so values published before that save are below S, and after
//! it the records either agree (S) or differ (S + 1). Second, a fallback:
//! if the device's record is whole, the larger is S, and S + 4 is past
//! S + 1; if the device's record fell back and the kind's holds S, the
//! larger is S again; if the device's fell back and the kind's never got S
//! (a cut), the larger is the device's previous generation P, the save
//! that wrote S started from a view of P or P + 1, so S is at most P + 2,
//! everything published is at most P + 3, and P + 4 is new. A view of
//! clean records names the records themselves, so reading them again
//! reports the same settings. Two records each without their newest slot
//! at once (two losses no save mended between) could repeat a generation.

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

/// How a settings record read: its generation, and whether it fell back
/// to its older slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordRead {
    pub generation: u64,
    pub slots: Slots,
}

/// Whether a record read its newest slot with the other in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slots {
    /// Both slots valid, or the other erased.
    Clean,
    /// The other slot unreadable: a newest slot lost, or a write of this
    /// record cut short.
    FellBack,
}

/// The generation a view reports, from the device's record and, for a
/// kind with a record of its own, the kind's (the module's docs give the
/// rule and why it never names two different settings).
pub const fn view_generation(device: RecordRead, kind: Option<RecordRead>) -> u64 {
    let (larger, apart, fell_back) = match kind {
        Some(kind) => (
            if kind.generation > device.generation {
                kind.generation
            } else {
                device.generation
            },
            kind.generation != device.generation,
            matches!(kind.slots, Slots::FellBack),
        ),
        None => (device.generation, false, false),
    };

    if fell_back || matches!(device.slots, Slots::FellBack) {
        larger.saturating_add(4)
    } else if apart {
        larger.saturating_add(1)
    } else {
        larger
    }
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
    /// change; 0 for defaults never saved.
    pub generation: u32,
    /// Whether NOR holds records this firmware can update: both, since one
    /// request may change either.
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
