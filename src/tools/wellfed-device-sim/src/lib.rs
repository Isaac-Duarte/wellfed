//! What every simulated Wellfed device shares, whatever its kind: the parts
//! of a device a simulator stands in for around its kind's own loop and
//! policy. A kind's simulator (`wellfed-feeder-sim` for the feeder) runs its
//! firmware's loop on a platform built from these, and adds what only its
//! kind has (the feeder's feeding owner, plans and statistics).
//!
//! - [`flash`]: the storage region, in memory or in a file, on the board's
//!   sector map.
//! - [`stores`]: the stores every device keeps (settings, Wi-Fi, time, the
//!   clients and the identity, the event ring with the firmware's read
//!   turns, the wall clock), their mailboxes, and the factory reset and
//!   handover an earlier boot left marked.
//! - [`sockets`]: the device API's connections, mDNS and NTP on the host.
//! - [`ble`]: setup mode's BLE link, as message queues.
//! - [`banks`] and [`updates`]: the two banks, the loader and KM0's trial,
//!   and the firmware's own update stage between them.
//! - `harness` (the `harness` feature, for tests only): a `LocalSet`
//!   runtime, the process hold, a relay that can stall, and a repeatable
//!   generator.

#![forbid(unsafe_code)]

pub mod banks;
pub mod ble;
pub mod flash;
#[cfg(feature = "harness")]
pub mod harness;
pub mod sockets;
pub mod stores;
pub mod updates;
