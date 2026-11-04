//! The fountain's pins, safe state and stock flash as recovered from stock
//! 0.0.17. Each value is pinned, so a change here is a deliberate one.

use wellfed_board_plwf106::stock::{self, Span};
use wellfed_board_plwf106::{MODEL_CODE, STORAGE_OFFSET, STORAGE_SECTORS, pins, safe};

#[test]
fn pins_are_pinned() {
    assert_eq!(MODEL_CODE, 2);
    assert_eq!(pins::PUMP, 30);
    assert_eq!(pins::BUTTON, 15);
    assert_eq!([pins::SCALE_CLOCK, pins::SCALE_DATA], [37, 38]);
    assert_eq!(pins::LED_NETWORK, [26, 28]);
    assert_eq!(
        [
            pins::LED_WATER_NORMAL,
            pins::LED_WATER_LOW,
            pins::LED_WATER_LACK,
            pins::LED_FILTER
        ],
        [19, 35, 25, 39]
    );
    assert_eq!(pins::UNKNOWN_HIGH, [58, 55, 54]);
    assert_eq!(pins::PUMP_BIT, 1 << 30);
}

/// A fault or a flash lease clears the pump enable and touches nothing else.
#[test]
fn the_safe_state_only_stops_the_pump() {
    assert_eq!(safe::PORT_A_HIGH, 0);
    assert_eq!(safe::PORT_A_LOW, 1 << 30);
    assert_eq!(safe::PORT_B_LOW, 0);
}

/// No Wellfed store until an allocation is reviewed for this model.
#[test]
fn there_is_no_store() {
    assert_eq!((STORAGE_OFFSET, STORAGE_SECTORS), (0, 0));
}

#[test]
fn the_stock_flash_is_pinned() {
    let at = |start, end| Span { start, end };

    assert_eq!(stock::FLASH_BYTES, 8 << 20);
    assert_eq!([stock::BANK0, stock::BANK1], [0x6000, 0x18_6000]);
    assert_eq!(stock::IMAGE_FOOTPRINT, 0xe_b000);
    assert_eq!(
        [stock::WEIGHT_ZERO, stock::WEIGHT_SCALE],
        [0x7c_e000, 0x7c_f000]
    );
    assert_eq!(
        stock::PRESERVED,
        [
            at(0x00_0000, 0x00_6000),
            at(0x00_6000, 0x0f_1000),
            at(0x18_2000, 0x18_5000),
            at(0x18_6000, 0x27_1000),
            at(0x3e_8000, 0x3e_c000),
            at(0x3e_d000, 0x3e_f000),
            at(0x3f_0000, 0x40_0000),
            at(0x40_0000, 0x40_1000),
            at(0x7c_b000, 0x7c_e000),
            at(0x7c_e000, 0x7c_f000),
            at(0x7c_f000, 0x7d_0000),
        ]
    );
}
