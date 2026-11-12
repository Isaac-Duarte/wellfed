//! The board a firmware is built for, and the contract every board crate
//! holds. The SDK (the AmebaD bindings, the chip HAL and its routines on both
//! cores), the kind drivers and the product read every board-specific value
//! from here, never from a board crate, so a second board supplies its own by
//! being selected instead.
//!
//! **One board per build, by `WELLFED_BOARD`.** The build script reads it
//! (`build/main.rs`): the reviewed build wrapper sets it per image from the
//! device it builds for, and the repository's cargo configuration sets the
//! PLAF103 for every other build. It must name exactly one known board, or
//! nothing builds. Code that only one board can run is gated on
//! `cfg(wellfed_board = "<name>")`, and code that needs the stores on
//! `cfg(wellfed_board_store)`, which crates that depend on this one get from
//! the same decision (`build/forward.rs`).
//!
//! **The contract is plain constants, not a trait.** Every value is used in
//! a constant context somewhere: a static's length (`STORAGE_SECTORS`), a
//! compile-time check, or the RAM-resident code of a flash lease, which must
//! stay leaf code with its masks as immediates. A trait's associated
//! constants would need a generic parameter through each of those, and a
//! static cannot be generic. So a board crate is a module tree of `const`s,
//! one is chosen at build time, and [`contract`] checks, at compile time,
//! that the chosen one has every item and type the contract names. What the
//! contract names is what the firmware uses, no more:
//!
//! - [`MODEL_CODE`]: the model, as `wellfed_proto::device::Model::code`
//!   numbers it (the SDK takes no product schema; the firmware checks it).
//! - [`STORAGE_OFFSET`] and [`STORAGE_SECTORS`]: the replacement firmware's
//!   storage region, zero sectors on a board with no reviewed allocation.
//! - `storage`, who owns each of the region's sectors, only on a board with
//!   a reviewed storage map (`cfg(wellfed_board_store)`).
//! - [`CONFIRM_SECTOR`]: the boot confirmation record's sector, if any.
//! - [`safe`]: the outputs' safe state a fault or a flash lease writes
//!   straight to the data registers.
//! - [`pins`]: the GPIO and ADC channels the drivers take, with
//!   `ADC_PADS`, the pads the ADC's opening shuts down, on every board
//!   (empty for none). Each other name is one
//!   a driver of this firmware reads; a board without that hardware leaves
//!   it out, and so does the firmware that cannot drive it.
#![no_std]

#[cfg(wellfed_board = "plaf103")]
pub use wellfed_board_plaf103::{MODEL_CODE, STORAGE_OFFSET, STORAGE_SECTORS, pins, safe, storage};

#[cfg(wellfed_board = "plwf106")]
pub use wellfed_board_plwf106::{MODEL_CODE, STORAGE_OFFSET, STORAGE_SECTORS, pins, safe};

/// The storage sector of the boot confirmation record, which the OTA trial
/// on both cores reads and KM4 writes once a build proves itself. `None` on
/// a board with no store: no build there can be confirmed, so the trial on
/// bank 1 always runs out, and no sector is invented for it.
#[cfg(wellfed_board_store)]
pub const CONFIRM_SECTOR: Option<u32> = Some(storage::CONFIRM.first);
#[cfg(not(wellfed_board_store))]
pub const CONFIRM_SECTOR: Option<u32> = None;

/// What every board must provide, checked against the chosen one at
/// compile time: a board crate missing an item, or giving it another type,
/// does not build.
mod contract {
    use super::{MODEL_CODE, STORAGE_OFFSET, STORAGE_SECTORS, pins, safe};

    const _: u8 = MODEL_CODE;
    const _: u32 = STORAGE_OFFSET;
    const _: u32 = STORAGE_SECTORS;
    const _: [u32; 3] = [safe::PORT_A_HIGH, safe::PORT_A_LOW, safe::PORT_B_LOW];
    // Every board opens the ADC (the entropy source reads the supply).
    const _: &[u32] = pins::ADC_PADS;

    // A storage map exactly when the build script says the board has one.
    #[cfg(wellfed_board_store)]
    const _: () = assert!(STORAGE_SECTORS > 0 && STORAGE_OFFSET.is_multiple_of(0x1000));
    #[cfg(not(wellfed_board_store))]
    const _: () = assert!(STORAGE_SECTORS == 0);

    /// What the shared stores reach: the boot confirmation record, which
    /// the OTA trial on both cores keeps, and what the settings writer's
    /// factory reset and handover reach: the reset's marker, the update
    /// store, and the sectors the reset and the handover clear, erased one
    /// step each by index.
    #[cfg(wellfed_board_store)]
    mod store {
        use crate::storage::{self, Sectors};

        const _: Sectors = storage::CONFIRM;
        const _: [Sectors; 2] = [storage::RESET, storage::UPDATE];
        const _: u32 = storage::FACTORY_CLEARED_SECTORS;
        const _: fn(u32) -> Option<u32> = storage::factory_cleared;
        const _: u32 = storage::HANDOVER_CLEARED_SECTORS;
        const _: fn(u32) -> Option<u32> = storage::handover_cleared;
    }
}
