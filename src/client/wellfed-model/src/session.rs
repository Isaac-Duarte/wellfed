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

/// How a feeder's API revision stands against a client's: what to tell the
/// person when something is missing. The API only grows, so either way every
/// op the client knows still goes ([device API evolution]); this only says
/// which side to update.
///
/// [device API evolution]: ../../../../docs/firmware/device-api.md#evolution
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Skew {
    Same,
    /// The feeder's build is older: an op it answers `Unavailable` needs
    /// the feeder updated.
    FeederBehind,
    /// The feeder's build is newer: what it added needs the client
    /// updated. Never a reason to offer the feeder an older build.
    FeederAhead,
}

impl Skew {
    /// How a feeder's `revision` stands against `ours`.
    pub fn of(revision: u32, ours: u32) -> Self {
        match revision.cmp(&ours) {
            core::cmp::Ordering::Less => Self::FeederBehind,
            core::cmp::Ordering::Equal => Self::Same,
            core::cmp::Ordering::Greater => Self::FeederAhead,
        }
    }
}

/// How an invite redemption ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Joined {
    /// The new key is enrolled with this role; it opens sessions now. The
    /// device names its model, as a session's welcome does.
    Enrolled {
        role: ClientRole,
        model: Model,
    },
    Refused(Refusal),
}

/// OpenAPI schemas for the wire's device names, which `wellfed-proto`
/// (`no_std`, with no schema derive) cannot carry itself: a field of
/// either type names its function in `#[schema(schema_with = ..)]`.
pub mod schema {
    use utoipa::openapi::schema::{Object, ObjectBuilder, Type};

    use super::{DeviceKind, Model};

    /// [`DeviceKind`] as its JSON spells it.
    pub fn device_kind() -> Object {
        ObjectBuilder::new()
            .schema_type(Type::String)
            .enum_values(Some(DeviceKind::ALL.map(DeviceKind::name)))
            .description(Some(
                "A kind of device, which one module of the device API serves.",
            ))
            .build()
    }

    /// [`Model`] as its JSON spells it.
    pub fn model() -> Object {
        ObjectBuilder::new()
            .schema_type(Type::String)
            .enum_values(Some(Model::ALL.map(Model::name)))
            .description(Some("A device model, which names its kind."))
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::Skew;

    /// The revision is an ordered count: the lower one is the older build,
    /// and the client's own is the reference.
    #[test]
    fn the_older_revision_is_the_side_to_update() {
        assert_eq!(Skew::of(1, 2), Skew::FeederBehind);
        assert_eq!(Skew::of(2, 2), Skew::Same);
        assert_eq!(Skew::of(3, 2), Skew::FeederAhead);
    }
}
