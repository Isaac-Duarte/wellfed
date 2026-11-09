//! Settings writer against the fake SDK storage, with the hooks already
//! armed (arming itself is covered by the confirmation and updater tests).
//!
//! One binary, one module per store or behaviour; `support` holds the fake
//! flash hooks and the helpers that start and step a writer.

mod calibration_plans_and_meal_call;
mod events;
mod handover;
mod keys;
mod records;
mod reset;
mod settings_wifi_and_time;
mod stats;
mod support;
mod turns_and_wakes;
mod written_events;
