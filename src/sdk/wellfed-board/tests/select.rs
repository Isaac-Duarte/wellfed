//! The board choice: exactly one known name chooses a board, and the crate
//! re-exports the board that name chose. Run under each board's
//! `WELLFED_BOARD` (`make check-rust` runs both).

#[path = "../build/select.rs"]
mod select;

#[test]
fn exactly_one_known_board_is_chosen() {
    assert_eq!(
        select::chosen(Some("plaf103")).map(|b| b.name),
        Some("plaf103")
    );
    assert_eq!(
        select::chosen(Some("plwf106")).map(|b| b.name),
        Some("plwf106")
    );

    for refused in [
        None,
        Some(""),
        Some("plaf103,plwf106"),
        Some("plaf103 plwf106"),
        Some("PLAF103"),
        Some("plaf203"),
    ] {
        assert!(select::chosen(refused).is_none(), "{refused:?}");
    }
}

/// Only the feeder has a reviewed storage map; the fountain's allocation is
/// still to be reviewed.
#[test]
fn only_reviewed_boards_have_a_store() {
    let stores: Vec<(&str, bool)> = select::BOARDS.iter().map(|b| (b.name, b.store)).collect();
    assert_eq!(stores, [("plaf103", true), ("plwf106", false)]);
    assert_eq!(select::names(), ["plaf103", "plwf106"]);
}

/// The build's board is the one `WELLFED_BOARD` named, by its model code.
#[test]
fn the_named_board_is_the_one_re_exported() {
    let expected = match env!("WELLFED_BOARD") {
        "plaf103" => 1,
        "plwf106" => 2,
        other => panic!("no board {other}"),
    };
    assert_eq!(wellfed_board::MODEL_CODE, expected);
}

#[cfg(wellfed_board = "plaf103")]
#[test]
fn the_feeder_confirms_in_its_reviewed_sector() {
    assert_eq!(wellfed_board::CONFIRM_SECTOR, Some(4));
    assert_eq!(wellfed_board::STORAGE_SECTORS, 48);
}

/// The fountain has no store, so nothing can name a confirmation sector and
/// every storage sector is out of bounds.
#[cfg(wellfed_board = "plwf106")]
#[test]
fn the_fountain_has_no_store() {
    assert_eq!(wellfed_board::CONFIRM_SECTOR, None);
    assert_eq!(wellfed_board::STORAGE_SECTORS, 0);
    assert_eq!(wellfed_board::safe::PORT_A_LOW, 1 << 30);
}
