//! The client side of the feeder's protocols ([API design]), sans-IO: the
//! caller moves bytes over BLE or a socket and this crate decides what they
//! say. The Android app (through generated bindings) and the owner's
//! `wellfed` CLI share it, so the two cannot disagree about the wire.
//!
//! It holds the BLE setup session ([`setup`]), the device API's sessions
//! and invite redemption ([`api`]), and the whole session every client
//! runs over them ([`connection`]: keepalive, answers, the event cursor and
//! LAN recovery), the update an owner pushes over it ([`update`]) and the
//! owner's own update-signing key it enrolls ([`update_key`]), a
//! feed's trace read a page at a time ([`trace`]), and the feeding history
//! the event history records ([`history`]). A release file the owner
//! keeps until they install it is checked with the feeder's own offer check
//! ([`release`]). The meal call's clip is converted from the owner's
//! recording and uploaded a chunk at a time ([`call`]). Editing the plans and
//! what they come to today are [`plans`], and a portion's weight
//! [`calibration`]. What a client shows of the feeder's day (the last feed,
//! today's meals, the next one and the portions fed) is [`today`], so the
//! app and Home Assistant agree.
//! The consumer installer's steps from stock to a confirmed replacement,
//! stock's setup frame and the backup gate before promotion are
//! [`install`]. How a write ended, by its answer, is [`write`], the one
//! reading those paths and every client share.
//!
//! Every public type is `wellfed-model`'s owned values or plain bytes (an
//! `Op` in; answers, pushed events, gaps and samples, welcomes, refusals and
//! setup replies out), from the lower-level [`api::Session`] the bench
//! drives up to [`connection::Connection`], so what crosses into the app's
//! bindings or the server has no lifetime tied to a frame buffer. The
//! wire's borrowed types stay inside the crate.
//!
//! [API design]: ../../../../docs/firmware/device-api.md
#![forbid(unsafe_code)]

pub mod alerts;
pub mod api;
pub mod calibration;
pub mod call;
pub mod connection;
pub mod history;
pub mod install;
pub mod plans;
pub mod release;
pub mod setup;
pub mod time;
pub mod today;
pub mod trace;
pub mod update;
pub mod update_key;
pub mod write;
