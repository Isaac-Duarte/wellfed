//! Which board a build is for: the one name `WELLFED_BOARD` holds. The build
//! script decides with it, and the crate's test checks the decision.

/// A board this checkout builds for.
pub(crate) struct Board {
    /// The model's name as `wellfed_proto::device::Model::name` spells it,
    /// which the reviewed build wrapper sets from the device it builds for.
    pub(crate) name: &'static str,
    /// Its board crate holds a reviewed storage map (`storage`), with the
    /// stores every device keeps. A board without one has a region of zero
    /// sectors, and code that names a store does not build for it.
    pub(crate) store: bool,
}

pub(crate) static BOARDS: [Board; 2] = [
    Board {
        name: "plaf103",
        store: true,
    },
    Board {
        name: "plwf106",
        store: false,
    },
];

/// The board `value` names. Exactly one name chooses a board: unset, empty,
/// a list or an unknown name chooses none, and the build stops.
pub(crate) fn chosen(value: Option<&str>) -> Option<&'static Board> {
    let value = value?;
    BOARDS.iter().find(|board| board.name == value)
}

/// Every board's name, for the cfg's declared values and the refusal.
pub(crate) fn names() -> Vec<&'static str> {
    BOARDS.iter().map(|board| board.name).collect()
}
