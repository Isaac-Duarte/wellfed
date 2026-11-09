//! The pack's floor for feeding.
//!
//! [`Floor`] judges the pack against [`FloorLimits`] in the unit the
//! feeding owner samples (stock-scaled millivolts at the pin). A pack below
//! `floor` for `hold_ms` in a row, or below `cutoff` at all, is below the
//! floor; it is above again only once it reads `clear` or more.
//! The hold lets a motor's start-up surge sag the pack for a poll or two
//! without stopping the feed; the cutoff stops it at once when the sag is
//! deep enough to threaten the regulator; the hysteresis keeps a pack that
//! recovers at rest from feeding, sagging and stopping in a loop.

use wellfed_core::interval_valid;

/// The pack's floor for feeding, in the feeding owner's sample unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FloorLimits {
    /// Below this for `hold_ms` in a row, the pack is below the floor.
    pub floor: u32,
    /// Once below, the pack is above again only at this or more.
    pub clear: u32,
    /// Below this, the pack is below the floor at once.
    pub cutoff: u32,
    pub hold_ms: u32,
}

impl FloorLimits {
    pub const fn valid(&self) -> bool {
        self.cutoff != 0
            && self.cutoff < self.floor
            && self.floor < self.clear
            && interval_valid(self.hold_ms)
    }
}

/// The pack against its floor. Unjudged until a reading at or above the
/// floor, a reading under the cutoff, or a hold under the floor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Floor {
    /// `None` until judged.
    below: Option<bool>,
    /// When the readings went under the floor (or missing), while they stay.
    under_since: Option<u32>,
}

impl Floor {
    /// One poll at `now_ms`: the pack's reading, or `None` when the
    /// acquisition failed, which counts as under the floor.
    pub fn update(&mut self, now_ms: u32, reading: Option<u32>, limits: &FloorLimits) {
        match reading {
            Some(value) if value >= limits.clear => {
                self.below = Some(false);
                self.under_since = None;
            }
            Some(value) if value >= limits.floor => {
                // Inside the band a judged pack keeps its verdict.
                self.below = Some(self.below.unwrap_or(false));
                self.under_since = None;
            }
            Some(value) if value < limits.cutoff => {
                self.below = Some(true);
                self.under_since.get_or_insert(now_ms);
            }
            Some(_) | None => {
                let since = *self.under_since.get_or_insert(now_ms);
                if now_ms.wrapping_sub(since) >= limits.hold_ms {
                    self.below = Some(true);
                }
            }
        }
    }

    /// Whether the pack is below its floor; `None` before it is judged.
    pub const fn below(&self) -> Option<bool> {
        self.below
    }

    /// Whether the pack may carry a feed: judged above the floor.
    pub const fn above(&self) -> bool {
        matches!(self.below, Some(false))
    }
}
