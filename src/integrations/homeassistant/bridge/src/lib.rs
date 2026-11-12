//! Home Assistant's Rust half. The integration reaches each feeder one of
//! two ways, chosen at setup: directly, as an ordinary device API client
//! with no server between them, or through the owner's server, under a
//! token of the owner's account. Everything the feeder or the server checks
//! (the Noise session, the invite exchange, request ids, the event cursor,
//! the server's API and its idempotency keys) is `wellfed-client`'s,
//! `wellfed-connect`'s and `wellfed-server-client`'s, so Home Assistant's
//! Python never holds a wire format or a key derivation of its own.
//!
//! - [`key`] makes or restores the integration's client key and redeems an
//!   invite with it.
//! - [`feeder`] runs one feeder's session on a thread of its own and hands
//!   its status and events out as [`api::Update`]s, over either route
//!   ([`server`] is the server's). A feed or lock change goes out once,
//!   under a fresh request id or idempotency key, and an answer that never
//!   came is reported as uncertain rather than sent again.
//! - [`server`] also signs in to the owner's server at setup, lists its
//!   feeders and revokes the token again.
//! - With the `python` feature, `python` exposes them as the
//!   `wellfed_homeassistant` module. Its calls block, so the integration
//!   runs them in Home Assistant's executor; each releases the interpreter
//!   while it waits.

#![cfg_attr(not(feature = "python"), forbid(unsafe_code))]

mod alerts;
pub mod api;
pub mod feeder;
pub mod key;
#[cfg(feature = "python")]
mod python;
pub mod server;
mod writes;

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
    /// A join went and its answer was lost, and a session then found the
    /// key unknown: the feeder did not take the invite.
    #[error(
        "the join's answer was lost, and the feeder does not know this key: it did not take the invite"
    )]
    JoinNotTaken,
    /// A join went and its answer was lost, and no session could say
    /// whether the feeder enrolled the key: it may have.
    #[error(
        "the join's answer was lost and a session to check it failed, so the key may be enrolled: {0}"
    )]
    JoinUnchecked(#[source] wellfed_connect::Error),
    #[error("start the session thread: {0}")]
    Thread(std::io::Error),
    #[error(transparent)]
    Server(#[from] wellfed_server_client::Error),
    #[error("a client key is 32 bytes, not {0}")]
    KeyLength(usize),
}
