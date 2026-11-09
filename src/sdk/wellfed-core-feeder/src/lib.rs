//! Hardware-independent feeder logic on the Wellfed SDK: what only a feeder
//! needs, beside the device-neutral `wellfed-core` that every kind builds
//! on.
//!
//! Feeding ([`feed`]: the controller, its inputs and outputs, the durable
//! interface to the journal and the synthetic check that drives them
//! together), the feed journal on NOR ([`feed_journal`]), the meal schedule
//! ([`schedule`]), the pack's floor for feeding ([`floor`]), the feeder's
//! own settings beside the device's ([`settings`]) with the desiccant
//! reminder's count ([`desiccant`]), and the feeding owner's UART records
//! as the host tools read them ([`record`]).
//!
//! `wellfed-core` never names anything here: where the SDK runs feeder
//! logic (the startup's synthetic check), it calls a hook the product's
//! runtime defines. States, faults and reasons follow core's rules: Rust
//! enums with fixed discriminants, a record's byte becoming a variant only
//! through `from_byte`.
#![no_std]
#![deny(unsafe_code)]

pub mod desiccant;
pub mod feed;
pub mod feed_journal;
pub mod floor;
pub mod record;
pub mod schedule;
pub mod settings;
