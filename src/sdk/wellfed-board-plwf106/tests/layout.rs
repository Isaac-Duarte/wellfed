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

/// The flash writers' copy of the stock flash, the model's `layout.json`,
/// describes the same flash as [`stock::PRESERVED`]. The writers keep the
/// banks by their own bounds instead (bank 0 is never written, bank 1 is
/// what they write), so every other span lies in one of the layout's
/// preserved ranges. Those ranges add only two sectors the board leaves out:
/// `0x3ec000`, which holds data of unknown role, and the erased `0x3ef000`,
/// so the settings sectors stay one range.
#[test]
fn the_flash_writers_layout_keeps_the_same_stock_flash() {
    let text = include_str!("../../../firmware/devices/plwf106/layout.json");
    let hex = |field: &str| -> Vec<u32> {
        text.split(&format!("\"{field}\": \"0x"))
            .skip(1)
            .map(|rest| u32::from_str_radix(&rest[..8], 16).unwrap())
            .collect()
    };
    let ranges: Vec<(u32, u32)> = hex("start").into_iter().zip(hex("end_exclusive")).collect();
    assert_eq!(ranges.len(), text.matches("\"name\"").count());
    assert!(text.contains(&format!(
        "\"candidate_payload_cap_bytes\": {},",
        stock::IMAGE_FOOTPRINT
    )));

    let banks = [stock::BANK0, stock::BANK1];
    let spans: Vec<Span> = stock::PRESERVED
        .into_iter()
        .filter(|span| !banks.contains(&span.start))
        .collect();
    assert_eq!(spans.len(), stock::PRESERVED.len() - 2);

    for span in &spans {
        assert!(
            ranges
                .iter()
                .any(|(s, e)| *s <= span.start && span.end <= *e),
            "{span:?}"
        );
    }
    let extra = [0x3e_c000, 0x3e_f000];
    for (start, end) in ranges {
        for sector in (start..end).step_by(0x1000) {
            let kept = spans.iter().any(|s| s.start <= sector && sector < s.end);
            assert!(kept || extra.contains(&sector), "{sector:#x}");
        }
    }
}
