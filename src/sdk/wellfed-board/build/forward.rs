//! The chosen board as cfgs. `wellfed-board`'s build script emits them for
//! itself and hands them on as `links` metadata; a crate that gates code on
//! the board includes this file in its build script and calls [`forward`],
//! so the choice is made and checked once, in `wellfed-board`'s.

/// Whether a board's crate holds a reviewed storage map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Store {
    Reviewed,
    /// No allocation reviewed yet: a region of zero sectors, and code that
    /// names a store does not build for the board.
    Unreviewed,
}

/// Emit `cfg(wellfed_board = "<name>")` and, for a board with a reviewed
/// storage map, `cfg(wellfed_board_store)`, with both declared over every
/// board's name (`names`, space-separated).
pub(crate) fn emit(name: &str, names: &str, store: Store) {
    let values: Vec<String> = names.split(' ').map(|name| format!("\"{name}\"")).collect();
    println!(
        "cargo::rustc-check-cfg=cfg(wellfed_board, values({}))",
        values.join(", ")
    );
    println!("cargo::rustc-check-cfg=cfg(wellfed_board_store)");
    println!("cargo::rustc-cfg=wellfed_board=\"{name}\"");
    if store == Store::Reviewed {
        println!("cargo::rustc-cfg=wellfed_board_store");
    }
}

/// [`emit`] for the board `wellfed-board` chose, from its metadata.
#[allow(
    dead_code,
    reason = "wellfed-board's own build script emits; only its dependents forward"
)]
pub(crate) fn forward() {
    let read = |key: &str| {
        let name = format!("DEP_WELLFED_BOARD_{key}");
        std::env::var(&name).unwrap_or_else(|_| panic!("wellfed-board sets {name}"))
    };
    let store = if read("STORE") == "1" {
        Store::Reviewed
    } else {
        Store::Unreviewed
    };
    emit(&read("NAME"), &read("NAMES"), store);
}
