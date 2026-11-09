//! Records kept on NOR that the SDK itself owns: the random seed and the
//! update store, each a dual-slot store ([`slot`]): two alternating erase
//! units under one torn-write rule, each store with its own fields and
//! recovery policy. A device kind's records are its own crates', in the
//! same dual-slot store: the feeder's journal is `wellfed_core_feeder`'s,
//! and the product's records (settings, Wi-Fi credentials, client keys)
//! `wellfed_device::store`'s, beside its event ring.

pub mod handover;
pub mod seed;
pub mod slot;
pub mod update;
