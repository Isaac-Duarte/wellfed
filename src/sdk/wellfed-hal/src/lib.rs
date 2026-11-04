//! EXPERIMENTAL AmebaD chip HAL: what a device of any kind needs on the
//! KM4, and the generic drivers (buttons, LEDs, buzzer, audio, ADC) that take
//! their pins from the board the firmware is built for ([`board`]). A
//! device kind's own hardware sits in its driver crate above this one (the
//! feeder's motor board and feeding owner in `wellfed-hal-feeder`).
//!
//! Host builds run against [`sdk::fake`]; KM4 target builds need the `sdk`
//! feature and the pinned SDK headers, both from `wellfed-amebad-sys`. The
//! routines both cores link (flash park, hooks, peer, OTA trial) live in
//! `wellfed-hal-common` and are re-exported here.
#![cfg_attr(target_os = "none", no_std)]
#![allow(clippy::missing_safety_doc)]

// The ports' public types carry SDK types, so the SDK crate is part of this API.
pub use wellfed_amebad_sys as sdk;
// The board the ports drive, through the board contract: its pins, its
// outputs' safe state and its storage sector map.
pub use wellfed_board as board;
pub use wellfed_hal_common::{cell, flash, trial};

#[cfg(all(target_os = "none", not(feature = "km4")))]
compile_error!("wellfed-hal target builds are KM4-only; KM0 links wellfed-hal-common");

pub mod leds;
pub mod startup;
pub mod stopped;

pub mod adc;
pub mod audio;
// Compiled for host tests, and on the target only for a Bluetooth
// firmware, whose image links the C peripheral it calls.
#[cfg(any(not(target_os = "none"), feature = "bluetooth"))]
pub mod ble;
pub mod bringup;
pub mod buttons;
pub mod buzzer;
pub mod clock;
pub mod entropy;
pub mod link;
pub mod rng;
pub mod settings;
pub mod writer_wake;

#[cfg(feature = "virtual-panel")]
mod virtual_panel;

pub mod flash_lock;
pub mod health;
pub mod interlock;
pub mod net_status;
pub mod ota;
pub mod power;
pub mod supervision;
