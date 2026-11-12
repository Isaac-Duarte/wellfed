//! The record stores every device keeps, on the Wellfed SDK runtime: the
//! glue between the shared schema (`wellfed_device::store`) and the SDK's
//! tasks, which a device kind's runtime builds on (`wellfed-feeder-runtime`
//! for the feeder).
//!
//! The SDK runtime runs a settings writer in its service task and joins
//! Wi-Fi from its C task on every firmware, but the stores they read and
//! write are the product's, which the SDK may not depend on. So the runtime
//! calls hooks the product defines:
//! - `wellfed_store_step`, the service task's settings stage: the
//!   [`writer`] over the device's stores and its kind's ([`writer::Product`]),
//!   which the kind's runtime instantiates and exports, and which alone
//!   writes the settings, Wi-Fi, client and event stores;
//! - `wellfed_wifi_run`, the C Wi-Fi task's body, which runs the SDK's
//!   driver (`wellfed_hal::link`) with this crate's choice between stored
//!   and build credentials ([`network`]), exported from here.
//!
//! The rest carries work between the loop task and the writer, each a
//! one-slot or bounded mailbox: client changes and the identity ([`keys`]),
//! Wi-Fi saves ([`network`]), the time settings ([`time`]) and the event
//! history ([`events`]). [`stress`] is the settings store's power-cut
//! probe, kept for its host tests.
//!
//! Host builds test everything against the SDK fake.
#![cfg_attr(target_os = "none", no_std)]

#[cfg(all(target_os = "none", not(feature = "km4")))]
compile_error!("wellfed-device-runtime target builds are KM4-only");

pub mod asked;
pub mod events;
pub mod keys;
pub mod network;
pub mod stress;
pub mod time;
pub mod writer;
