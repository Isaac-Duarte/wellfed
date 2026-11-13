//! Device API, the device's side ([`wellfed_proto::api`]).
//!
//! A [`Server`] holds [`SESSIONS`] fixed slots, one per TCP connection, and
//! never touches a socket: the firmware's driver copies received bytes into
//! a slot ([`Server::rx`], [`Server::received`]), sends what a slot holds
//! ([`Server::tx`], [`Server::sent`]) and closes it when asked
//! ([`Server::closing`]). Everything else happens in [`Server::step`], once
//! per pass of the loop task:
//! - **One handshake operation per pass,** across all slots: an X25519
//!   costs about 27 ms on the KM4, so four clients connecting at once
//!   cannot stack up to hold the loop. While a feed runs only IK session
//!   handshakes go on, a refused one closes unanswered, and after one is
//!   refused or fails the rest wait for the feed's end; the loop steps its
//!   feeding owner again after each operation ([`Server::handshook`]).
//! - **One frame per slot per pass,** and a slot writes again only once its
//!   last frame is sent, so a flood costs only its own turn.
//! - **Every write has an id** the server remembers per client key, so a
//!   retry, even after a reconnect, gets the first answer instead of
//!   running again. A write the device finishes later is remembered from
//!   the moment it is queued, so a retry on any session waits for that
//!   write rather than running it again (refused `Stale` by its own
//!   change, or `Busy` behind it), and a write whose session ended first
//!   is settled by the pass, so its answer is what the device did then.
//! - **Writes carry their base generation;** a stale one is refused with
//!   the current generation.
//! - **A feed is answered once it is durably reserved,** with the number
//!   the journal holds it under, or with its refusal if it waits for storage
//!   writes past their deadline (`Server::feed_refused`), when a later feed
//!   takes that number; its result is a `Fed` event. A feed is asked of
//!   the device only while its [`FeederDevice::readiness`] is ready, which
//!   `Status` also reports; a restart is refused while a feed is in flight.
//! - **Updates are an owner's.** The device
//!   checks an offer outside the loop (its signatures need more stack than
//!   the loop has) and answers on a later pass. An update request waits
//!   for no feed: an offer, a cancel or a key statement is refused `Busy`
//!   while a feed is queued or runs, and a feed is refused `Busy` while a
//!   request or an install holds the flash. Chunks are
//!   idempotent by offset, so they are not remembered writes. An update
//!   key is enrolled only once someone at the feeder presses Dispense.
//! - **The debug log is an owner's:** after
//!   `TailLog`, a session is pushed the lines the device's RAM tail keeps,
//!   in batches that fit a frame, only once its last frame is out and
//!   after any event or sample waiting.
//!
//! Sessions authenticate with Noise IK under the device's identity, against
//! the enrolled clients the settings writer publishes. A revoked key's
//! sessions close as soon as the new table arrives. A new client redeems an
//! invite with XXpsk3 and is enrolled through the keys mailbox.
//!
//! The welcome names the device's kind and model ([`Device::model`]). The
//! shared core (status, Wi-Fi, clients, time, updates, events, the log and
//! statistics) asks the rest of the device through [`Device`], which the
//! server is generic over. A feeder request goes to the feeder's module,
//! which asks through the [`FeederDevice`] that [`Device::feeder`] hands
//! it, generic so every call stays direct; a device of another kind names
//! [`NoFeeder`] and hands none, and the module's requests are refused
//! `Unavailable`. The firmware implements both, and the host tests
//! a fake of them.
//!
//! The crate is not yet kind-neutral: its [`mailbox`] carries the feeder's
//! settings and plans (`wellfed-core-feeder`'s types), so every crate that
//! links it, `wellfed-device` and `wellfed-device-runtime` among them,
//! links the feeder's core too. Only the feeder has a network image, so no
//! image pays for it; the shared stores' vocabulary moves out when a second
//! kind gets one.
//!
//! Beside the server sit what its requests hand the rest of the feeder
//! ([`mailbox`]), who may become a client and when ([`pairing`]), the setup
//! exchange that BLE and the recovery window share ([`setup`]), and the
//! client and Wi-Fi values the feeder's stores keep ([`clients`], [`wifi`]).
//! Like the feeder's product crate above it, it knows no hardware, keeps no
//! clock and allocates nothing. The `fake` feature adds the host tests'
//! fake device (`fake`), which allocates; no firmware enables it.
#![no_std]
#![deny(unsafe_code)]

#[cfg(feature = "fake")]
extern crate alloc;

pub mod clients;
mod device;
#[cfg(feature = "fake")]
pub mod fake;
pub mod mailbox;
pub mod pairing;
mod record;
mod server;
pub mod setup;
pub mod wifi;

pub use device::{
    Device, EventRead, FeederDevice, KeyChange, KeyOutcome, NoFeeder, Reader, UpdateAnswer,
    UpdateKeyChange, UpdateRequest,
};
pub use record::{Reason, Record};
pub use server::handshake::{fingerprint, key_prefix};
pub use server::request::proto_role;
pub use server::{Pass, Server};

use wellfed_proto::api::{FRAME_MAX, MESSAGE_MAX, UPDATE_CHUNK_MAX};
use wellfed_session::frame;

/// Connections served at once (lwIP leaves room for four).
pub const SESSIONS: usize = 4;
/// Write ids remembered per client.
pub const REMEMBERED: usize = 32;
/// A connection that has not finished its handshake by then is dropped.
pub const HANDSHAKE_MS: u32 = 10_000;
/// A session that sends nothing for this long is dropped; clients send the
/// frozen `Alive` as a keepalive, which every build answers.
pub const IDLE_MS: u32 = 90_000;
/// A connection whose queued bytes do not move for this long is dropped, and
/// its slot freed: its client has stopped reading, and is no longer listening
/// for whatever it would be told next. It reconnects as after any lost
/// connection, and a retried write id gets its first answer.
pub const STALL_MS: u32 = 30_000;
/// After `Reboot`, long enough for the answer to leave.
pub const REBOOT_MS: u32 = 1_000;

const BUFFER: usize = frame::PREFIX + FRAME_MAX;

// A whole chunk request seals into one frame: the request id, the op's tag,
// its offset and the data's length prefix, with room to spare.
const _: () = assert!(UPDATE_CHUNK_MAX + 16 + 16 <= MESSAGE_MAX);
