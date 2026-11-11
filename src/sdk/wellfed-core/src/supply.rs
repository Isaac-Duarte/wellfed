//! The supply: what a power sample says about the adapter and the battery
//! pack, with each kind of uncertainty named. The pack's floor for feeding
//! is the feeder's (`wellfed_core_feeder::floor`).
//!
//! **Readings.** A board samples its pack through a divider and its adapter
//! on a detect pin. [`classify`] turns the latest sample into a [`Reading`]
//! that never passes an unknown off as a value: no sample at all is
//! [`Pack::Unread`], an old one [`Pack::Stale`] (and the adapter unknown
//! with it), and a value no pack of the board's can read is
//! [`Pack::Implausible`], kept for the record but never judged.
//!
//! **The charge.** A plausible reading carries a percentage on the
//! board's [`Curve`]: stock's lookup, linear in the pin's millivolts from
//! `empty` (0 %) to `full` (100 %) in 10 % steps, each reading reporting the
//! first step at or above it, clamped to 0–100.

/// What a board's pack readings are judged against.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PackLimits {
    /// Pack millivolts per thousand pin millivolts.
    pub divider_per_mille: u32,
    /// Below this at the pin no pack is fitted.
    pub absent_pin_mv: u32,
    /// A pack reading above this is implausible for the board's cells.
    pub max_pack_mv: u32,
    /// On the pack alone, a reading below this is implausible: the board
    /// would not be running to take it.
    pub min_running_mv: u32,
    /// A sample at least this old is stale.
    pub stale_ms: u32,
    /// The pin's millivolts against the pack's charge.
    pub curve: Curve,
}

/// The pack's charge against the pin's millivolts, in 10 % steps: stock's
/// battery lookup. A reading at or below `empty` is 0 %, and each step of
/// `(full - empty) / 10` above it adds 10 %, rounded up, so a reading
/// reports the first step whose threshold is at or above it; at or above
/// `full` it is 100 %.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Curve {
    pub empty_pin_mv: u32,
    pub full_pin_mv: u32,
}

impl Curve {
    /// The charge, 0–100 in steps of 10, at `pin_mv`. A curve with no span
    /// reads 0 % up to `empty` and 100 % above it.
    pub fn percent(&self, pin_mv: u32) -> u8 {
        if pin_mv <= self.empty_pin_mv {
            return 0;
        }
        let span = self.full_pin_mv.saturating_sub(self.empty_pin_mv);
        let above = pin_mv - self.empty_pin_mv;
        if span == 0 || above >= span {
            return 100;
        }
        // Rounded up to the next step; `above < span`, so 1 to 10 steps.
        let steps = above.saturating_mul(10).div_ceil(span);
        u8::try_from(steps.saturating_mul(10)).map_or(100, |percent| percent.min(100))
    }
}

impl PackLimits {
    /// The pack's millivolts from the pin's, `None` when no pack is fitted.
    pub const fn pack_mv(&self, pin_mv: u32) -> Option<u32> {
        if pin_mv < self.absent_pin_mv {
            return None;
        }
        Some(pin_mv.saturating_mul(self.divider_per_mille) / 1000)
    }
}

/// The latest supply sample, however old.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub pin_mv: u32,
    pub adapter: bool,
    /// Milliseconds since it was taken.
    pub age_ms: u32,
}

/// The battery pack as a sample shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pack {
    /// No pack fitted: the pin reads near zero.
    Absent,
    /// The pack's millivolts, and its charge on the board's [`Curve`].
    Reading { mv: u16, percent: u8 },
    /// A value outside what the board's pack can read, as read.
    Implausible { mv: u16 },
    /// Nothing sampled: the monitor has not run, its ADC failed, or the
    /// firmware does not sample the supply.
    Unread,
    /// The last sample is this old.
    Stale { age_ms: u32 },
}

/// A sample, judged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reading {
    /// The adapter's detect pin; `None` with no fresh sample.
    pub adapter: Option<bool>,
    pub pack: Pack,
}

impl Reading {
    pub const UNREAD: Self = Self {
        adapter: None,
        pack: Pack::Unread,
    };

    /// The pack's millivolts when they are a plausible fresh reading.
    pub const fn pack_mv(&self) -> Option<u16> {
        match self.pack {
            Pack::Reading { mv, .. } => Some(mv),
            Pack::Absent | Pack::Implausible { .. } | Pack::Unread | Pack::Stale { .. } => None,
        }
    }
}

/// Judge the latest `sample` against the board's `limits`.
pub fn classify(sample: Option<Sample>, limits: &PackLimits) -> Reading {
    let Some(sample) = sample else {
        return Reading::UNREAD;
    };
    if sample.age_ms >= limits.stale_ms {
        return Reading {
            adapter: None,
            pack: Pack::Stale {
                age_ms: sample.age_ms,
            },
        };
    }

    let pack = if let Some(mv) = limits.pack_mv(sample.pin_mv) {
        let mv16 = u16::try_from(mv).unwrap_or(u16::MAX);
        let too_low = !sample.adapter && mv < limits.min_running_mv;
        if mv > limits.max_pack_mv || too_low {
            Pack::Implausible { mv: mv16 }
        } else {
            Pack::Reading {
                mv: mv16,
                percent: limits.curve.percent(sample.pin_mv),
            }
        }
    } else {
        Pack::Absent
    };
    Reading {
        adapter: Some(sample.adapter),
        pack,
    }
}
