//! The supply in the loop: what the device API reports of it, what its
//! changes leave in the history, battery mode, and the hold a brownout puts
//! on feeding.
//!
//! **The reading.** Each pass judges the platform's latest sample
//! ([`wellfed_core::supply`]) with the feeding owner's verdict on the
//! pack's floor ([`Feeds::below_floor`]) into one [`Supply`]: the live
//! `Supply` metric, the status and the events carry it as it is, unknowns
//! named, with the pack's charge on stock's curve.
//!
//! **No cells.** On the adapter a pack reads its own voltage and an empty
//! holder reads about 16 mV at the pin, so "no pack" is told from "a flat
//! pack" by [`Pack::Absent`]'s threshold, and a pack that is not there has
//! no floor to be below: [`supply`] reports no verdict for it, whatever
//! the owner made of the near-zero channel. Cutting the adapter breaks
//! that reading: the decaying 5 V rail shows on the channel as a plausible
//! pack for the moment before the board stops. Cells cannot be fitted to a
//! board in that moment, so [`Watch::cells`] keeps "no pack" across it,
//! until the channel has read something else for [`CELLS_MS`].
//!
//! **Events.** The adapter coming or going and the pack crossing its floor
//! are recorded as `Supply` events, at most one per [`EVENT_MS`]: a change
//! inside that window is recorded once it ends, as the supply then stands,
//! so an adapter that chatters or a pack that hovers at its floor costs one
//! event a minute, not one a change. The supply a boot first reads is its
//! baseline, not an event (the status and the metric report it); the first
//! change from it is recorded at once. A sample too old to trust (adapter
//! unknown) changes nothing here: staleness is reported, not recorded.
//!
//! **Battery mode** is stock's: once the adapter has been gone for
//! [`RADIO_OFF_MS`] on a pack that reads, the radio goes off ([`Radio`]),
//! so no client or server reaches the feeder until the adapter returns,
//! when it comes back on at once. Meals, the buttons and the lights carry
//! on; nothing else in the loop changes. Only a fresh sample moves the
//! mode: an unknown adapter, or a pack that reads absent or implausible on
//! the pack alone (the board could not be running on it), neither starts
//! nor cancels the wait. That keeps a power cut from counting as battery
//! mode: with no cells fitted, the decaying 5 V rail reads as a plausible
//! pack with the adapter gone, for the moment before the board stops,
//! well inside the wait. Setup mode keeps the radio on, since the vendor's
//! BLE stack runs only while the WLAN driver is up. Once the radio has
//! come back on it stays on for at least [`RADIO_MIN_ON_MS`], so an
//! adapter that chatters cannot cycle the WLAN driver (its heap, its
//! firmware load) every few seconds. When the radio goes off the loop
//! also stops an install that is receiving: no chunk could reach it.
//!
//! **Brownout.** A boot whose reset was a brownout (the ROM's flag) holds
//! feeding as `Blocked::Battery`. The pack recovers at rest, so its reading
//! at once would let the next feed start, and the drive could sag it into
//! the same brownout again: a loop of resets that each leave a feed
//! uncertain, and that the storm breaker counts. The hold ends when the
//! adapter is seen, or once the pack has read at least [`RELEASE_PERCENT`]
//! on every fresh sample for [`RELEASE_MS`]: fresh cells, which read
//! 100 %, so a battery-only feeder feeds again without a power cycle. A
//! reading under it, or any sample that is not a fresh plausible reading,
//! starts the wait again, so a rail decaying at a cut never lasts long
//! enough to lift it. A power cycle (new cells) is a power-on reset, not a
//! brownout, and never holds.

use core::fmt;

pub use wellfed_core::liveness::Radio;
use wellfed_core::supply::{Pack, Reading};
use wellfed_proto::api::{Battery, Power, ResetCause, Supply};

/// The least time between two `Supply` events.
pub const EVENT_MS: u32 = 60_000;

/// How long the adapter stays gone before the radio goes off: stock's 5 s.
pub const RADIO_OFF_MS: u32 = 5_000;

/// The least time the radio stays on after battery mode brought it back,
/// before battery mode may turn it off again.
pub const RADIO_MIN_ON_MS: u32 = 60_000;

/// The charge a pack must read to lift a brownout's hold: stock's 50 %
/// step, a pin reading above 1890 mV, the floor (1650) plus 240 mV, or a
/// pack of about 3.9 V (1.3 V a cell) at rest. A pack that browned out
/// and rebounds at rest stays well under it; fresh cells read 100 %.
pub const RELEASE_PERCENT: u8 = 50;

/// How long every fresh reading must stay at [`RELEASE_PERCENT`] or more:
/// three minutes, 36 of the monitor's 5 s samples and far more of the
/// feeding owner's.
pub const RELEASE_MS: u32 = 180_000;

/// How long the channel must read something else, after the adapter read
/// no pack, before it is believed. A board with no cells does not run this
/// long past a cut (the radio's own wait, [`RADIO_OFF_MS`], rests on the same
/// fact), and cells fitted on the adapter showing this much later costs
/// nothing.
pub const CELLS_MS: u32 = RADIO_OFF_MS;

/// The wire's form of a judged reading, with the owner's verdict. A pack
/// that is not there is not below anything: the owner judges the channel
/// (near zero, so under its cutoff) with or without one.
pub fn supply(reading: Reading, below_floor: Option<bool>) -> Supply {
    let below_floor = if matches!(reading.pack, Pack::Absent) {
        None
    } else {
        below_floor
    };
    let battery = match reading.pack {
        Pack::Absent => Battery::Absent,
        Pack::Reading { mv, percent } => Battery::Reading { mv, percent },
        Pack::Implausible { mv } => Battery::Implausible { mv },
        Pack::Unread => Battery::Unread,
        Pack::Stale { age_ms } => Battery::Stale { age_ms },
    };
    Supply {
        adapter: reading.adapter,
        battery,
        below_floor,
    }
}

/// The raw reading as the judged supply allows: no millivolts for a pack
/// judged absent, so the rail at a cut never shows as a number here either.
pub const fn power(power: Power, battery: Battery) -> Power {
    match battery {
        Battery::Absent => Power {
            battery_mv: None,
            ..power
        },
        _ => power,
    }
}

/// What lifted a brownout's hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Released {
    /// The adapter was seen.
    Adapter,
    /// The pack read at least [`RELEASE_PERCENT`] for [`RELEASE_MS`].
    Pack,
}

impl Released {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Adapter => "adapter",
            Self::Pack => "pack",
        }
    }
}

/// What the supply's watch did, for the record lines (`PLBAT2`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Record {
    /// The radio was asked on or off.
    Radio(Radio),
    /// This boot followed a brownout: feeding holds.
    Held,
    Released(Released),
    /// Battery mode stopped an install that was receiving; `stopped` is
    /// false when the stage did not take the cancel.
    InstallStopped {
        stopped: bool,
    },
}

/// One record's body (`PLBAT2`).
pub fn describe(record: Record, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match record {
        Record::Radio(radio) => write!(f, "radio {}", radio.name()),
        Record::Held => f.write_str("hold brownout"),
        Record::Released(by) => write!(f, "hold released by={}", by.name()),
        Record::InstallStopped { stopped } => {
            write!(f, "install stop queued={}", u8::from(stopped))
        }
    }
}

/// What a `Supply` event is recorded for: the adapter and the floor's
/// verdict, not the millivolts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Recorded {
    adapter: bool,
    below_floor: Option<bool>,
    /// `None` for the boot's baseline, which was not an event.
    at_ms: Option<u32>,
}

/// Where the feeder stands between the adapter and its batteries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// On the adapter, or not yet known to be off it.
    Mains,
    /// The adapter has been gone, with a pack reading, since `since_ms`.
    Leaving { since_ms: u32 },
    /// On the batteries: the radio is off.
    Battery,
}

/// The supply's history, battery mode and the brownout hold, kept by the
/// loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Watch {
    /// `None` until the first pass reads this boot's reset cause.
    brownout: Option<bool>,
    /// While held: since when every fresh reading on the pack has been at
    /// [`RELEASE_PERCENT`] or more.
    recovered_since: Option<u32>,
    recorded: Option<Recorded>,
    mode: Mode,
    /// The radio last asked for.
    radio: Option<Radio>,
    /// When battery mode last brought the radio back on.
    radio_on_ms: Option<u32>,
    no_cells: Option<NoCells>,
}

/// The adapter's samples read no pack, so what the channel shows next is
/// not yet a pack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NoCells {
    /// Since when the channel has read something else.
    other_since: Option<u32>,
}

impl Default for Watch {
    fn default() -> Self {
        Self::new()
    }
}

impl Watch {
    pub const fn new() -> Self {
        Self {
            brownout: None,
            recovered_since: None,
            recorded: None,
            mode: Mode::Mains,
            radio: None,
            radio_on_ms: None,
            no_cells: None,
        }
    }

    /// `supply` as the rest of the loop and the device API should see it:
    /// no pack while the adapter's last word was that there is none, and
    /// the channel has read otherwise for less than [`CELLS_MS`]. Taken
    /// before [`Self::note`] on the same reading.
    pub fn cells(&mut self, now_ms: u32, supply: Supply) -> Supply {
        // No fresh sample says nothing either way.
        let Some(adapter) = supply.adapter else {
            return supply;
        };
        if supply.battery == Battery::Absent {
            if adapter || self.no_cells.is_some() {
                self.no_cells = Some(NoCells { other_since: None });
            }
            return supply;
        }

        let Some(no_cells) = self.no_cells.as_mut() else {
            return supply;
        };
        let since = *no_cells.other_since.get_or_insert(now_ms);
        if now_ms.wrapping_sub(since) >= CELLS_MS {
            self.no_cells = None;
            return supply;
        }
        Supply {
            battery: Battery::Absent,
            below_floor: None,
            ..supply
        }
    }

    /// Note the supply at `now_ms` on a boot that `reset` started: battery
    /// mode moves, and the brownout hold lifts once the adapter is seen or
    /// the pack has recovered. The supply to record as a `Supply` event,
    /// when a change is due.
    pub fn note(
        &mut self,
        now_ms: u32,
        reset: ResetCause,
        supply: Supply,
        mut record: impl FnMut(Record),
    ) -> Option<Supply> {
        if self.brownout.is_none() && reset == ResetCause::Brownout {
            record(Record::Held);
        }
        self.brownout.get_or_insert(reset == ResetCause::Brownout);
        self.mode = self.mode.step(now_ms, supply);
        if let Some(by) = self.release(now_ms, supply) {
            self.brownout = Some(false);
            self.recovered_since = None;
            record(Record::Released(by));
        }

        let adapter = supply.adapter?;
        let Some(last) = self.recorded.as_mut() else {
            self.recorded = Some(Recorded {
                adapter,
                below_floor: supply.below_floor,
                at_ms: None,
            });
            return None;
        };
        // The owner's first verdict after the baseline, when it finds the
        // pack above its floor, is no change.
        if last.below_floor.is_none() && supply.below_floor == Some(false) {
            last.below_floor = Some(false);
        }
        let changed = last.adapter != adapter
            || (supply.below_floor.is_some() && supply.below_floor != last.below_floor);
        let due = last
            .at_ms
            .is_none_or(|at| now_ms.wrapping_sub(at) >= EVENT_MS);

        if !(changed && due) {
            return None;
        }
        self.recorded = Some(Recorded {
            adapter,
            below_floor: supply.below_floor,
            at_ms: Some(now_ms),
        });
        Some(supply)
    }

    /// While held, whether this reading lifts the hold.
    fn release(&mut self, now_ms: u32, supply: Supply) -> Option<Released> {
        if !self.holds() {
            return None;
        }
        if supply.adapter == Some(true) {
            return Some(Released::Adapter);
        }

        let recovered = supply.adapter == Some(false)
            && matches!(supply.battery, Battery::Reading { percent, .. } if percent >= RELEASE_PERCENT);
        if !recovered {
            self.recovered_since = None;
            return None;
        }
        let since = *self.recovered_since.get_or_insert(now_ms);
        (now_ms.wrapping_sub(since) >= RELEASE_MS).then_some(Released::Pack)
    }

    /// Whether this boot followed a brownout and neither the adapter nor
    /// a recovered pack has lifted it since: feeding holds as
    /// `Blocked::Battery`.
    pub fn holds(&self) -> bool {
        self.brownout == Some(true)
    }

    /// Whether the feeder runs on its batteries: the adapter has been gone
    /// for [`RADIO_OFF_MS`].
    pub fn on_battery(&self) -> bool {
        self.mode == Mode::Battery
    }

    /// The radio this pass asks for at `now_ms`; `setup` (BLE setup mode
    /// runs) keeps it on, and so does a radio back on for less than
    /// [`RADIO_MIN_ON_MS`]. Recorded when it changes.
    pub fn radio(&mut self, now_ms: u32, setup: bool, mut record: impl FnMut(Record)) -> Radio {
        let settled = self
            .radio_on_ms
            .is_none_or(|on| now_ms.wrapping_sub(on) >= RADIO_MIN_ON_MS);
        let radio = if self.on_battery() && !setup && settled {
            Radio::Off
        } else {
            Radio::On
        };

        // The boot starts with the radio on: only a change is a record.
        let last = self.radio.unwrap_or(Radio::On);
        if last != radio {
            record(Record::Radio(radio));
        }
        if last == Radio::Off && radio == Radio::On {
            self.radio_on_ms = Some(now_ms);
        } else if settled {
            // Past the minimum: forget it, so the uptime's wrap never
            // brings it back.
            self.radio_on_ms = None;
        }
        self.radio = Some(radio);
        radio
    }
}

impl Mode {
    fn step(self, now_ms: u32, supply: Supply) -> Self {
        match (supply.adapter, supply.battery) {
            (Some(true), _) => Self::Mains,
            (Some(false), Battery::Reading { .. }) => match self {
                Self::Mains => Self::Leaving { since_ms: now_ms },
                Self::Leaving { since_ms } if now_ms.wrapping_sub(since_ms) >= RADIO_OFF_MS => {
                    Self::Battery
                }
                other => other,
            },
            // An unknown adapter, or a pack that could not be running the
            // board on its own, says nothing about battery mode.
            _ => self,
        }
    }
}
