//! The fountain's pins, safe state and stock flash as recovered from stock
//! 0.0.17. Each value is pinned, so a change here is a deliberate one.

use std::collections::BTreeSet;

use wellfed_board_plwf106::stock::{self, Span};
use wellfed_board_plwf106::{MODEL_CODE, STORAGE_OFFSET, STORAGE_SECTORS, pins, safe};
use wellfed_formats::layout::{Hex, Layout};

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
            at(0x3e_8000, 0x3f_0000),
            at(0x3f_0000, 0x40_0000),
            at(0x40_0000, 0x40_1000),
            at(0x7c_b000, 0x7c_e000),
            at(0x7c_e000, 0x7c_f000),
            at(0x7c_f000, 0x7d_0000),
        ]
    );
}

/// The flash writers' copy of the stock flash, the model's `layout.json`,
/// keeps exactly the sectors [`stock::PRESERVED`] does, apart from the two
/// banks: the writers keep those by their own bounds instead (bank 0 is
/// never written, bank 1 is what they write). The cap is the board's
/// reference footprint.
#[test]
fn the_flash_writers_layout_keeps_the_same_stock_flash() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../firmware/devices/plwf106/layout.json"
    );
    let layout: Layout = wellfed_formats::load(std::path::Path::new(path)).unwrap();
    assert_eq!(layout.model, "PLWF106");
    assert_eq!(
        layout.candidate_payload_cap_bytes,
        u64::from(stock::IMAGE_FOOTPRINT)
    );
    assert_eq!(
        layout.ota_bank_offsets,
        [Hex(u64::from(stock::BANK0)), Hex(u64::from(stock::BANK1))]
    );

    let sectors = |spans: &mut dyn Iterator<Item = (u64, u64)>| -> BTreeSet<u64> {
        spans
            .flat_map(|(start, end)| (start..end).step_by(0x1000))
            .collect()
    };
    let banks = [stock::BANK0, stock::BANK1];
    let board = sectors(
        &mut stock::PRESERVED
            .into_iter()
            .filter(|span| !banks.contains(&span.start))
            .map(|span| (u64::from(span.start), u64::from(span.end))),
    );
    let writers = sectors(
        &mut layout
            .known_preserve_ranges
            .iter()
            .map(|range| (range.start.0, range.end_exclusive.0)),
    );
    assert_eq!(board, writers);
}
