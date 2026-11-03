//! Opening a session: what a client says, what the device answers, and
//! how an invite redemption ends. The device's welcome names its kind and
//! model ([`DeviceKind`], [`Model`]), which say what it serves.

use serde::{Deserialize, Serialize};

use crate::{ClientRole, Refusal};
pub use wellfed_proto::device::{DeviceKind, Model};

/// What a client says about itself when it opens a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hello {
    /// The protocol's major version.
    pub major: u16,
}

impl Hello {
    /// This crate's protocol.
    pub const CURRENT: Self = Self {
        major: wellfed_proto::api::PROTOCOL_MAJOR,
    };
}

/// Why the feeder refused to open a session, so a client can say which
/// side needs what.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenRefusal {
    /// The key is not enrolled (or was revoked).
    Unknown,
    /// The client's major version is one this feeder does not serve;
    /// `supported` is the newest it does.
    Version { supported: u16 },
    /// Every session slot is taken.
    Full,
}

/// How an invite redemption ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Joined {
    /// The new key is enrolled with this role; it opens sessions now.
    Enrolled {
        role: ClientRole,
    },
    Refused(Refusal),
}
