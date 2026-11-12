//! What stock PLAF103 1.1.15 speaks, and only what installing from it and
//! rolling back to it need: its MQTT
//! protocol, the broker it connects to, the setup frame, the image server it
//! downloads a replacement from, and the ledger that sends each install
//! command at most once. The replacement firmware speaks no MQTT; its
//! updates and status go over its device API.
//!
//! [`protocol`] is the PETLIBRO cloud protocol every stock model shares,
//! with each model's message set beside it ([`protocol::plaf103`], the only
//! one so far): another model brings its own set, not a fork of the core.
//!
//! [`server`] runs stock's side in the caller's process: the broker, the
//! session with stock and, optionally, the loopback API in [`api`]. [`ota`]'s
//! image server runs the same way. The Android app (through
//! `wellfed-install`) starts both as tokio tasks it owns and stops; the
//! bench's `wellfed-stock` binary (`wellfed-stock-server`, a host tool) calls
//! the same entry points, and its `serve` runs [`server`] with the trial
//! disabled (docs/development/stock-server.md).

#![forbid(unsafe_code)]
#![warn(clippy::print_stdout)]

pub mod api;
pub mod build_info;
pub mod clock;
pub mod config;
pub mod controller;
pub mod enroll;
pub mod mqtt;
pub mod ota;
pub mod protocol;
pub mod server;
pub mod setup;
pub mod storage;

/// After a failed `accept`: the listener goes on, since one failure must not
/// end a listener an install depends on. A connection the peer reset before
/// it was accepted costs nothing; running out of descriptors or memory gets
/// a short pause first, so the loop does not spin while it lasts.
pub(crate) async fn accept_failed(error: &std::io::Error) {
    tracing::warn!(error = %error, "accept failed; listening on");
    if let Some(pause) = accept_pause(error) {
        tokio::time::sleep(pause).await;
    }
}

fn accept_pause(error: &std::io::Error) -> Option<std::time::Duration> {
    use std::io::ErrorKind;

    match error.kind() {
        ErrorKind::ConnectionAborted | ErrorKind::ConnectionReset | ErrorKind::Interrupted => None,
        _ => Some(std::time::Duration::from_millis(100)),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Error, ErrorKind};

    use super::accept_pause;

    #[test]
    fn a_reset_peer_goes_on_at_once_and_exhaustion_pauses() {
        assert_eq!(
            accept_pause(&Error::from(ErrorKind::ConnectionAborted)),
            None
        );
        // EMFILE: the process is out of descriptors until some close.
        assert!(accept_pause(&Error::from_raw_os_error(24)).is_some());
    }
}
