//! The feeder's side of the board's power: the pack's floor the feeding
//! owner holds a feed to ([power](../../../../docs/firmware/power.md)). The
//! chip HAL's power monitor (`wellfed_hal::power`) samples and judges the
//! pack for every device; holding a feed to it is the feeder's.

use wellfed_core_feeder::floor::FloorLimits;
use wellfed_hal::ota::confirm::HEALTHY_MS;
use wellfed_hal::power::PACK;

/// The pack's floor for feeding, in the battery channel's scaled unit (pin
/// millivolts; pack volts are about 2.063 times them). Provisional: no
/// motor has run on the pack yet, and the regulator's dropout and the SoC's
/// brownout level are unmeasured on this board.
/// - `floor` 1650 (pack about 3.40 V, 1.13 V a cell): stock's 0 % point,
///   below which stock stops its outputs on battery. A fresh pack reads
///   about 2306 (4.77 V) at rest.
/// - `clear` 1710 (about 3.53 V): stock's 10 % step above it, so a pack
///   that recovers at rest after a sag does not feed again until it has
///   recovered by a step.
/// - `cutoff` 1550 (about 3.20 V): a sag this deep stops a feed at once;
///   it leaves the 3.3 V rail's regulator little headroom, and a brownout
///   mid-drive would leave the feed uncertain.
/// - `hold_ms` 300, timed from the first reading under the floor: fifteen of
///   the owner's 20 ms polls in a feed, three of its 100 ms battery
///   conversions at rest. The motor's start-up surge lasted one poll on the
///   bench (on the adapter); a sag under the floor that outlasts the hold is
///   the pack failing under load.
pub const FLOOR: FloorLimits = FloorLimits {
    floor: 1650,
    clear: 1710,
    cutoff: 1550,
    hold_ms: 300,
};

// The floor is stock's 0 % point: the charge the status reports reads 0 %
// where feeding stops.
const _: () = assert!(PACK.curve.empty_pin_mv == FLOOR.floor);
const _: () = assert!(FLOOR.valid());

// Lit emitters restart the confirmation's health wait, so retried food
// readings must leave it room.
const _: () = assert!(wellfed_core_feeder::feed::food_read::FOOD_READ_RETRY_MS > HEALTHY_MS);
