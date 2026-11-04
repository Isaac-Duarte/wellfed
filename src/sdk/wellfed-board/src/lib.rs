//! The board a firmware is built for, and the contract every board crate
//! holds. The SDK (the AmebaD bindings, the chip HAL and its routines on both
//! cores), the feeder drivers and the product read every board-specific value
//! from here, never from a board crate, so a second board supplies its own by
//! being selected instead.
//!
//! **The contract is plain constants, not a trait.** Every value is used in
//! a constant context somewhere: a static's length (`STORAGE_SECTORS`), a
//! compile-time check, or the RAM-resident code of a flash lease, which must
//! stay leaf code with its masks as immediates. A trait's associated
//! constants would need a generic parameter through each of those, and a
//! static cannot be generic. So a board crate is a module tree of `const`s,
//! one is chosen at build time by feature, and [`contract`] checks, at
//! compile time, that the chosen one has every item and type the contract
//! names. What the contract names is what the firmware uses, no more:
//!
//! - [`MODEL_CODE`]: the model, as `wellfed_proto::device::Model::code`
//!   numbers it (the SDK takes no product schema; the firmware checks it).
//! - [`STORAGE_OFFSET`] and [`STORAGE_SECTORS`]: the replacement firmware's
//!   storage region, and [`storage`], who owns each of its sectors.
//! - [`safe`]: the outputs' safe state a fault or a flash lease writes
//!   straight to the data registers.
//! - [`pins`]: the GPIO and ADC channels the drivers take. Each name is one
//!   a driver of this firmware reads; a board without that hardware leaves
//!   it out, and so does the firmware that cannot drive it.
#![no_std]

#[cfg(not(feature = "plaf103"))]
compile_error!("wellfed-board needs exactly one board feature: plaf103");

#[cfg(feature = "plaf103")]
pub use wellfed_board_plaf103::{MODEL_CODE, STORAGE_OFFSET, STORAGE_SECTORS, pins, safe, storage};

/// What every board must provide, checked against the chosen one at
/// compile time: a board crate missing an item, or giving it another type,
/// does not build.
mod contract {
    use super::storage::Sectors;
    use super::{MODEL_CODE, STORAGE_OFFSET, STORAGE_SECTORS, safe};

    const _: u8 = MODEL_CODE;
    const _: u32 = STORAGE_OFFSET;
    const _: u32 = STORAGE_SECTORS;
    const _: [u32; 3] = [safe::PORT_A_HIGH, safe::PORT_A_LOW, safe::PORT_B_LOW];
    // The boot confirmation record, which the OTA trial on both cores keeps.
    const _: Sectors = super::storage::CONFIRM;
}
