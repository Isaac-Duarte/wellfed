//! Chooses the board from `WELLFED_BOARD`, which names exactly one. The
//! reviewed build wrapper sets it per image from the device it builds for;
//! `.cargo/config.toml` sets the PLAF103 for every other build in the
//! repository, so a workspace build and its tests are the feeder's.
//!
//! Emits `cfg(wellfed_board = "<name>")`, and `cfg(wellfed_board_store)` for
//! a board with a reviewed storage map, then hands both to the crates that
//! gate code on the board through `links` metadata. An environment
//! variable, unlike a Cargo feature, holds one value per build: a workspace
//! build cannot turn on two boards.

mod forward;
mod select;

fn main() {
    println!("cargo::rerun-if-env-changed=WELLFED_BOARD");
    let value = std::env::var("WELLFED_BOARD").ok();
    let names = select::names().join(" ");
    let Some(board) = select::chosen(value.as_deref()) else {
        panic!("WELLFED_BOARD must name exactly one board of: {names}; it is {value:?}");
    };

    forward::emit(board.name, &names, board.store);
    println!("cargo::metadata=name={}", board.name);
    println!("cargo::metadata=names={names}");
    println!("cargo::metadata=store={}", u8::from(board.store));
}
