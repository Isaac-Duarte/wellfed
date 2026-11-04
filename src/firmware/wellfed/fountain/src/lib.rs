//! EXPERIMENTAL boot test for the PLWF106 fountain: the smallest Wellfed
//! image that says whether the factory loader runs our firmware from bank 1.
//!
//! It latches the pump enable (PA30) LOW before making it an output, prints
//! the banner, the build, the bank and the pump's readback on the log UART,
//! then one task beats every [`boot::BEAT_MS`]. It configures no other pin,
//! and links no Wi-Fi, BLE, store, updater or trial confirmation: it is not
//! on the SDK runtime, whose updater and stores need a reviewed allocation
//! this board does not have. It arms no watchdog. Every path leaves the
//! pump LOW: nothing drives it HIGH, and a panic clears it again
//! (`wellfed_board::safe`).
//!
//! The crate builds only for the fountain's board (`WELLFED_BOARD=plwf106`);
//! on any other it is empty, so a workspace build of the feeder's board
//! compiles it to nothing. Host builds test it against the SDK fake.
#![cfg_attr(target_os = "none", no_std)]

#[cfg(all(target_os = "none", not(feature = "km4")))]
compile_error!("wellfed-fw-fountain target builds are KM4-only");
#[cfg(all(target_os = "none", not(wellfed_board = "plwf106")))]
compile_error!(
    "wellfed-fw-fountain is the PLWF106's boot test: build it with WELLFED_BOARD=plwf106"
);

#[cfg(all(target_os = "none", wellfed_board = "plwf106"))]
mod app;
#[cfg(wellfed_board = "plwf106")]
pub mod boot;
