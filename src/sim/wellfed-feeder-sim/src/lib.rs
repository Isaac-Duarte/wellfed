//! A simulated feeder that serves the real device API over TCP, for the
//! server, the app and the CLI to work against without hardware.
//!
//! It runs the feeder's own code: the product's loop pass
//! (`wellfed_feeder::feeder`, the device API's driver and server, the
//! panel, pairing and setup mode, the event recorder) and the product's
//! stores on a flash image laid out like the feeder's storage region. What
//! it fakes is what the board provides ([`platform`]): the buttons (the
//! operator's [`console`]), the supply and signal it samples, the settings
//! writer's turn and the time. Nothing moves: its feeding owner
//! ([`feeding`]) keeps the feeder's own feed journal and times each feed,
//! so feeds, their numbers and their `Fed` events are the firmware's. It takes updates
//! as the feeder does, through the firmware's own update stage on two
//! simulated banks, with the loader's choice and KM0's trial around them
//! ([`banks`]).
//!
//! What every device shares (the storage image, the device's own stores
//! and their mailboxes, the host sockets, setup's BLE link, the banks and
//! the update stage) is `wellfed_device_sim`'s; this crate adds the
//! feeder's own around the feeder's loop.
//!
//! - [`feeding`]: the feeding owner, on the feed journal.
//! - [`platform`]: the loop's platform, over the stores and host sockets.
//! - [`sim`]: one feeder: the loop's pass, the writer's and update stage's
//!   turns.
//! - [`serve`]: the listener, and the loop that drives it.
//! - `harness` (the `harness` feature, for tests only): a simulator on a
//!   test's `LocalSet`, with `wellfed_device_sim`'s relay, runtime and
//!   generator.

#![forbid(unsafe_code)]

// The device's storage image and banks, as this simulator's users name them.
pub use wellfed_device_sim::{banks, flash};

pub mod console;
pub mod feeding;
#[cfg(feature = "harness")]
pub mod harness;
pub mod platform;
pub mod serve;
pub mod sim;
