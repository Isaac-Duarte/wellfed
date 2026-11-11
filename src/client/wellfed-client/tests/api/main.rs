//! The device API end to end on the host: `wellfed-client`'s sessions
//! and connections against the feeder's `Server` (`wellfed-device-api`),
//! with a fake device standing in for the mailboxes and the settings
//! writer. Each test moves bytes between the two by hand, so it controls
//! every pass and, for a connection, the client's clock.
//!
//! One binary, one module per behaviour; `support` holds the fake device,
//! the rig and the helpers that pump bytes between them.

mod diagnostics;
mod events_and_cursors;
mod feeds_and_refusals;
mod invites_recovery_and_keys;
mod retries;
mod revisions_and_leniency;
mod sessions_and_handshakes;
mod support;
mod updates;
mod writes;
