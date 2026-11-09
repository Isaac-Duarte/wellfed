//! Tests for the pack's floor for feeding: its hold, cutoff and hysteresis.

use wellfed_core_feeder::floor::{Floor, FloorLimits};

const FLOOR: FloorLimits = FloorLimits {
    floor: 1650,
    clear: 1710,
    cutoff: 1550,
    hold_ms: 300,
};

#[test]
fn the_floor_holds_a_sag_and_cuts_off_a_deep_one() {
    assert!(FLOOR.valid());
    let mut floor = Floor::default();
    assert_eq!(floor.below(), None);
    assert!(!floor.above());

    floor.update(0, Some(2000), &FLOOR);
    assert!(floor.above());

    floor.update(20, Some(1600), &FLOOR);
    floor.update(319, Some(1600), &FLOOR);
    assert!(floor.above(), "inside the hold");
    floor.update(320, Some(1600), &FLOOR);
    assert_eq!(floor.below(), Some(true));

    floor.update(340, Some(1700), &FLOOR);
    assert_eq!(floor.below(), Some(true), "inside the band");
    floor.update(360, Some(1710), &FLOOR);
    assert!(floor.above());

    floor.update(380, Some(1549), &FLOOR);
    assert_eq!(floor.below(), Some(true), "under the cutoff");
}

#[test]
fn a_first_reading_judges_the_pack() {
    let mut floor = Floor::default();
    floor.update(0, Some(1660), &FLOOR);
    assert!(
        floor.above(),
        "the band counts as above until a pack is low"
    );

    let mut floor = Floor::default();
    floor.update(0, Some(1600), &FLOOR);
    assert_eq!(floor.below(), None, "unjudged until the hold");
    floor.update(300, None, &FLOOR);
    assert_eq!(floor.below(), Some(true), "a missing reading counts under");
}

#[test]
fn the_hold_survives_the_clock_wrap() {
    let mut floor = Floor::default();
    floor.update(u32::MAX - 100, Some(2000), &FLOOR);
    floor.update(u32::MAX - 10, Some(1600), &FLOOR);
    floor.update(200, Some(1600), &FLOOR);
    assert!(floor.above());
    floor.update(290, Some(1600), &FLOOR);
    assert_eq!(floor.below(), Some(true));
}

#[test]
fn limits_must_order_cutoff_floor_and_clear() {
    for limits in [
        FloorLimits { cutoff: 0, ..FLOOR },
        FloorLimits {
            cutoff: 1650,
            ..FLOOR
        },
        FloorLimits {
            clear: 1650,
            ..FLOOR
        },
        FloorLimits {
            hold_ms: 0,
            ..FLOOR
        },
    ] {
        assert!(!limits.valid());
    }
}
