//! The owner's client key for the feeder's protocols: the `wellfed`
//! CLI creates it when asked to and records each feeder that enrolled it;
//! the sessions read it. The secret never leaves the owner's data
//! directory.
//!
//! Every reader goes through [`load`] or [`read`], the one place the
//! version is checked: a build that changes the shape keeps the previous
//! version's reader there and converts, so an owner who updates keeps
//! their key.

use std::path::Path;

use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;

/// Why a key file does not read.
#[derive(Debug, thiserror::Error)]
pub enum OwnerError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("key file version {found}, where this build reads {reads}")]
    Version { found: u32, reads: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerKeyFile {
    pub version: u32,
    /// The client's X25519 static secret, hex.
    pub secret: String,
    /// How feeders list this client: 1-16 bytes.
    pub label: String,
    /// The feeders that enrolled this key.
    pub feeders: Vec<EnrolledFeeder>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnrolledFeeder {
    /// The feeder's X25519 identity public key, hex. Its first 8 bytes are
    /// the fingerprint the feeder logs (`PLID1`).
    pub identity: String,
}

/// What every version shares.
#[derive(Deserialize)]
struct Header {
    version: u32,
}

/// The key file `bytes` hold, at this build's version. No earlier version
/// is read yet: none has shipped.
pub fn read(bytes: &[u8]) -> Result<OwnerKeyFile, OwnerError> {
    let Header { version } = serde_json::from_slice(bytes)?;
    crate::require(version == VERSION, || OwnerError::Version {
        found: version,
        reads: VERSION,
    })?;
    Ok(serde_json::from_slice(bytes)?)
}

/// The key file at `path`.
pub fn load(path: &Path) -> Result<OwnerKeyFile, OwnerError> {
    read(&std::fs::read(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_this_version_reads_and_every_field_is_required() {
        let file = br#"{"version":1,"secret":"00","label":"x","feeders":[]}"#;
        assert_eq!(read(file).unwrap().label, "x");

        assert!(matches!(
            read(br#"{"version":2,"secret":"00","label":"x","feeders":[]}"#),
            Err(OwnerError::Version { found: 2, reads: 1 })
        ));
        assert!(matches!(
            read(br#"{"version":1,"secret":"00","label":"x"}"#),
            Err(OwnerError::Json(_))
        ));
    }
}
