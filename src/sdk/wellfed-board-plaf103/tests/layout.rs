//! The storage map and pins as every firmware has written and driven them.
//! A store's sectors are on-flash state a reflash keeps, so a change here
//! strands records; this pins each value, not just the map's own consistency.

use wellfed_board_plaf103::storage::{self, Sectors};
use wellfed_board_plaf103::{STORAGE_OFFSET, STORAGE_SECTORS, pins};

const fn at(first: u32, count: u32) -> Sectors {
    Sectors { first, count }
}

#[test]
fn storage_region_and_sector_map_are_pinned() {
    assert_eq!(STORAGE_OFFSET, 0x34_0000);
    assert_eq!(STORAGE_SECTORS, 50);
    // Grown upward only: 0x340000 stays the first sector, and the end stays
    // under stock's settings at 0x3e8000.
    assert_eq!(STORAGE_OFFSET + STORAGE_SECTORS * 0x1000, 0x37_2000);

    assert_eq!(storage::JOURNAL, at(0, 2));
    assert_eq!(storage::SETTINGS, at(2, 2));
    assert_eq!(storage::CONFIRM, at(4, 1));
    assert_eq!(storage::NET, at(5, 2));
    assert_eq!(storage::SEED, at(7, 2));
    assert_eq!(storage::CLIENTS, at(9, 2));
    assert_eq!(storage::PLANS, at(11, 2));
    assert_eq!(storage::EVENTS, at(13, 12));
    assert_eq!(storage::UPDATE, at(25, 2));
    assert_eq!(storage::TIME, at(27, 2));
    assert_eq!(storage::TRACES, at(29, 3));
    assert_eq!(storage::STATS, at(32, 2));
    assert_eq!(storage::CALIBRATION, at(34, 2));
    assert_eq!(storage::RESET, at(36, 1));
    assert_eq!(storage::UPLINK, at(37, 3));
    assert_eq!(storage::CALL, at(40, 8));
    assert_eq!(storage::FEEDER_SETTINGS, at(48, 2));

    assert_eq!(
        storage::ALL,
        [
            storage::JOURNAL,
            storage::SETTINGS,
            storage::CONFIRM,
            storage::NET,
            storage::SEED,
            storage::CLIENTS,
            storage::PLANS,
            storage::EVENTS,
            storage::UPDATE,
            storage::TIME,
            storage::TRACES,
            storage::STATS,
            storage::CALIBRATION,
            storage::RESET,
            storage::UPLINK,
            storage::CALL,
            storage::FEEDER_SETTINGS,
        ]
    );
}

/// The factory reset keeps exactly the stores the owner decided (the
/// journal, calibration, update store, confirmation and seed, and the
/// statistics, which it rewrites keeping the erase counts) and erases
/// every other sector, clients first: the uplink store's reserved sectors
/// and the feeder's settings among them, and whatever a later build
/// allocates.
#[test]
fn the_factory_reset_clears_every_sector_it_does_not_keep() {
    let cleared: Vec<u32> = (0..storage::FACTORY_CLEARED_SECTORS)
        .map(|index| storage::factory_cleared(index).unwrap())
        .collect();
    assert_eq!(
        cleared,
        [9, 10, 2, 3, 5, 6, 11, 12]
            .into_iter()
            .chain(13..25)
            .chain(27..32)
            .chain(37..50)
            .collect::<Vec<u32>>()
    );
    assert_eq!(
        storage::factory_cleared(storage::FACTORY_CLEARED_SECTORS),
        None
    );

    for kept in [
        storage::JOURNAL,
        storage::CONFIRM,
        storage::SEED,
        storage::UPDATE,
        storage::CALIBRATION,
        storage::STATS,
        storage::RESET,
    ] {
        for sector in kept.first..kept.first + kept.count {
            assert!(!cleared.contains(&sector), "sector {sector} is kept");
        }
    }
    let kept: u32 = storage::FACTORY_KEPT.iter().map(|s| s.count).sum();
    assert_eq!(
        kept + storage::FACTORY_CLEARED_SECTORS,
        wellfed_board_plaf103::STORAGE_SECTORS
    );
}

/// The handover from stock erases the Wi-Fi, the plans and the uplink
/// store's sectors, in region order, and nothing it keeps or clears in a
/// step of its own.
#[test]
fn the_handover_clears_every_sector_it_does_not_keep() {
    let cleared: Vec<u32> = (0..storage::HANDOVER_CLEARED_SECTORS)
        .map(|index| storage::handover_cleared(index).unwrap())
        .collect();
    assert_eq!(cleared, [5, 6, 11, 12, 37, 38, 39]);
    assert!(
        storage::HANDOVER_KEPT.contains(&storage::FEEDER_SETTINGS),
        "the feeder's settings stay with the device's"
    );
    assert_eq!(
        storage::handover_cleared(storage::HANDOVER_CLEARED_SECTORS),
        None
    );
}

#[test]
fn pins_are_pinned() {
    assert_eq!([pins::MOTOR_A, pins::MOTOR_B], [13, 14]);
    assert_eq!([pins::IR_PA, pins::IR_PB], [15, 35]);
    assert_eq!(
        [pins::OUTLET, pins::FOOD, pins::FEEDBACK, pins::ADAPTER],
        [16, 17, 18, 19]
    );
    assert_eq!(pins::BUTTONS, [36, 37, 38, 39]);
    assert_eq!(
        [
            pins::LED_CONNECTION,
            pins::LED_FOOD,
            pins::LED_LOCK,
            pins::LED_FAULT
        ],
        [0, 2, 4, 54]
    );
    assert_eq!(pins::AMP, 55);
    assert_eq!([pins::ADC_CURRENT, pins::ADC_BATTERY], [4, 5]);

    assert_eq!(pins::MOTOR_MASK, 0x6000);
    assert_eq!([pins::MOTOR_A_BIT, pins::MOTOR_B_BIT], [1 << 13, 1 << 14]);
    assert_eq!([pins::IR_PA_BIT, pins::IR_PB_BIT], [1 << 15, 1 << 3]);
}

/// The flash writers' copy of the stock flash, `layout.json`, and the
/// region agree: the store lies above a full-size bank 1 and its candidate
/// cap, and clear of every range the writers preserve.
#[test]
fn the_storage_region_is_clear_of_the_flash_writers_layout() {
    use wellfed_formats::layout::Layout;

    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../firmware/devices/plaf103/layout.json"
    );
    let layout: Layout = wellfed_formats::load(std::path::Path::new(path)).unwrap();
    let start = u64::from(STORAGE_OFFSET);
    let end = start + u64::from(STORAGE_SECTORS) * layout.sector_bytes;

    let [bank0, bank1] = [layout.ota_bank_offsets[0].0, layout.ota_bank_offsets[1].0];
    assert!(start >= bank1 + (bank1 - bank0), "past a full-size bank 1");
    assert!(start >= bank1 + layout.candidate_payload_cap_bytes);
    for range in &layout.known_preserve_ranges {
        assert!(
            end <= range.start.0 || start >= range.end_exclusive.0,
            "{} overlaps the store",
            range.name
        );
    }
}
