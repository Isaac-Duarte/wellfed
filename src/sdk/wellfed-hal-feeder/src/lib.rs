//! EXPERIMENTAL feeder drivers on the AmebaD chip HAL (`wellfed-hal`): what
//! only a feeder has. The motor, optics and sensor board ([`board`]), its
//! timer tick and feedback capture ([`events`]), and the feeding owner that
//! drives a feed under supervision ([`owner`]). The chip HAL keeps
//! everything a device of any kind needs (startup, supervision and the
//! watchdog, the outputs' stopped state, flash leases, updates, the link, the
//! panel's buttons, LEDs and buzzer, the ADC), and both read their pins from
//! the board the firmware is built for (`wellfed-board`).
//!
//! The bench variants' injections live here too: a motor STOP that does not
//! take (`inject`, the `stop-fail` feature) and jams and sensor faults shown
//! to the input capture (`feed_faults`, the `feed-faults` feature). Neither
//! is ever in a product image.
#![cfg_attr(target_os = "none", no_std)]
#![allow(clippy::missing_safety_doc)]

pub mod board;
pub mod events;
pub mod owner;

// The `stop-fail` bench variant's STOP that does not take; never in a
// product image.
#[cfg(feature = "stop-fail")]
pub mod inject;
// The `feed-jam` and `sensor-fault` bench variants' jams and sensor
// faults; never in a product image.
#[cfg(feature = "feed-faults")]
pub mod feed_faults;
