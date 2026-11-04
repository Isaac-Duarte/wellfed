//! The policy every Wellfed device shares, whatever its kind: the records
//! it keeps on NOR ([`store`]) and the update view it serves ([`update`]).
//! It knows no hardware and keeps no clock, so a firmware runs it on the SDK
//! (`wellfed-device-runtime`) and the host tests it; a kind's own policy
//! (`wellfed-feeder` for the feeder) sits beside it, never below.
#![no_std]
#![deny(unsafe_code)]

pub mod store;
pub mod update;
