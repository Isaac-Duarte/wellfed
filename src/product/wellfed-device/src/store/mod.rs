//! The records every device keeps on NOR: its settings, Wi-Fi credentials,
//! the client keys and the time settings, each a dual-slot store
//! (`wellfed_core::store::slot`: two alternating erase units under one
//! torn-write rule, with the store's own fields and recovery policy), the
//! event history and the traces, each in a ring of units ([`events`],
//! [`traces`]), with the turns the event ring's readers take ([`reads`]),
//! and the factory reset's marker ([`reset`]). Each keeps its sectors in the
//! board's map and its on-flash layout as the firmware has always written
//! it. A device kind's own records (the feeder's plans, calibration, call
//! and statistics) live with its policy.
//!
//! A ring is not a dual-slot store: it appends records with no commit
//! marker, and a torn one simply fails its checksum and is skipped, so it
//! shares only the slot layer's I/O trait, checksum and field helpers.

pub mod clients;
pub mod events;
pub mod net;
pub mod reads;
pub mod reset;
pub mod settings;
pub mod time;
pub mod traces;
