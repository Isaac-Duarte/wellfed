//! The feeder's own records on NOR: the lifetime statistics, the portion
//! calibration, the feeding plans and the feeder's settings, each a
//! dual-slot store
//! (`wellfed_core::store::slot`: two alternating erase units under one
//! torn-write rule, with the store's own fields and recovery policy), and
//! the meal call ([`call`]): one clip across eight units, written once a
//! chunk at a time with its header last. The stores every device keeps
//! (settings, Wi-Fi, clients, time, the event and trace rings and the
//! factory reset's marker) are `wellfed_device::store`'s. Each keeps its
//! sectors in the board's map and its on-flash layout as the firmware has
//! always written it.

pub mod calibration;
pub mod call;
pub mod plans;
pub mod settings;
pub mod stats;
