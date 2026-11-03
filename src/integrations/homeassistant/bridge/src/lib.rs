//! Home Assistant's Rust half. The integration pairs with a feeder as an
//! ordinary device API client and holds one session to it, with no server
//! between them. Everything the feeder checks (the Noise session, the
//! invite exchange, request ids, the event cursor) is `wellfed-client`'s
//! and `wellfed-connect`'s, so Home Assistant's Python never holds a wire
//! format or a key derivation of its own.
//!
//! - [`key`] makes or restores the integration's client key and redeems an
//!   invite with it.
//! - [`feeder`] runs one feeder's session on a thread of its own and hands
//!   its status and events out as [`feeder::Update`]s. A feed or lock
//!   change goes out once, under a fresh request id, and an answer that
//!   never came is reported as uncertain rather than sent again.
//! - With the `python` feature, `python` exposes both as the
//!   `wellfed_homeassistant` module. Its calls block, so the integration
//!   runs them in Home Assistant's executor; each releases the interpreter
//!   while it waits.

#![cfg_attr(not(feature = "python"), forbid(unsafe_code))]

pub mod api;
pub mod feeder;
pub mod key;
#[cfg(feature = "python")]
mod python;
pub mod releases;
pub mod writes;

/// Why a call into the bridge failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("read the system random source: {0}")]
    Random(#[from] wellfed_formats::Error),
    #[error("a client key secret is 32 bytes, not {0}")]
    SecretLength(usize),
    #[error("a feeder identity is 32 bytes, not {0}")]
    IdentityLength(usize),
    #[error("not a Wellfed invite: {0}")]
    Invite(#[from] wellfed_client::api::InviteText),
    #[error(transparent)]
    Connect(#[from] wellfed_connect::Error),
    #[error("the feeder refused: {0:?}")]
    Refused(wellfed_model::Refusal),
    #[error("start the session thread: {0}")]
    Thread(std::io::Error),
    #[error(transparent)]
    Releases(#[from] wellfed_releases::Error),
    #[error("a release signing key is 32 bytes, not {0}")]
    KeyLength(usize),
    #[error("not a release id: 64 hex digits")]
    ReleaseId,
    #[error("read the release file: {0}")]
    File(std::io::Error),
    #[error("{0} bytes is longer than any release")]
    FileTooLarge(u64),
}
