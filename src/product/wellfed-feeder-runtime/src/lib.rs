//! The feeder's record stores on the Wellfed SDK runtime: the glue between
//! the feeder's schema (`wellfed_feeder::store`) and the SDK's tasks, over
//! the runtime every device shares (`wellfed_device_runtime`).
//!
//! The SDK runtime runs a settings writer in its service task on every
//! firmware, but the stores it writes are the product's, which the SDK may
//! not depend on. So the runtime calls `wellfed_store_step`, which this
//! crate defines over the shared writer and the feeder's stores
//! ([`writer`]), and every PLAF103 firmware links it
//! (`use wellfed_feeder_runtime as _;`), which keeps the owner's settings,
//! keys and history across a firmware swap. The Wi-Fi task's hooks come
//! with it, from `wellfed_device_runtime::network`, and so does the
//! startup's synthetic check of the feeder's logic,
//! `wellfed_startup_check` ([`diagnostic`]).
//!
//! The rest carries the feeder's work between the loop task and the
//! writer, each a one-slot mailbox: the settings, the device's with the
//! feeder's own ([`settings`]), the portion calibration
//! ([`calibration`]), the feeding plans ([`plans`]), the meal call's upload
//! ([`call`]), the statistics' totals and the event ring's fill
//! ([`stats`]), and the latest feed's trace on its way to storage
//! ([`traces`]).
//!
//! Host builds test everything against the SDK fake.
#![cfg_attr(target_os = "none", no_std)]
#![allow(clippy::missing_safety_doc)]

#[cfg(all(target_os = "none", not(feature = "km4")))]
compile_error!("wellfed-feeder-runtime target builds are KM4-only");

pub mod calibration;
pub mod call;
pub mod diagnostic;
pub mod plans;
pub mod settings;
pub mod stats;
pub mod traces;
pub mod writer;
