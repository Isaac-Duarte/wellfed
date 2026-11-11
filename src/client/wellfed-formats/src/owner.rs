//! The owner's client key for the feeder's protocols: the `wellfed`
//! CLI creates it when asked to and records each feeder that enrolled it;
//! the sessions read it. The secret never leaves the owner's data
//! directory.
//!
//! A file an owner keeps ([`crate::kept`]): every reader goes through
//! [`load`] or [`read`] and every writer through [`save`]. A key file of a
//! newer major is refused and never written over, and one that does not
//! read is refused rather than replaced, since a new key would be one no
//! feeder knows.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::kept::{self, Found, Kept};

/// The key file's major.
pub const VERSION: u32 = 1;

/// Why a key file does not read or was not written.
#[derive(Debug, thiserror::Error)]
pub enum OwnerError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Unreadable(#[from] kept::Unreadable),
    #[error(
        "the key file is version {major}, a newer build's (this one reads up to {reads}); it is left as it is"
    )]
    Newer { major: u32, reads: u32 },
    #[error("{0}")]
    Write(#[from] kept::WriteRefused),
}

/// No field is denied: a newer minor's additions are skipped.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OwnerKeyFile {
    pub version: u32,
    /// The client's X25519 static secret, hex.
    pub secret: String,
    /// How feeders list this client: 1-16 bytes.
    pub label: String,
    /// The feeders that enrolled this key.
    pub feeders: Vec<EnrolledFeeder>,
}

impl Kept for OwnerKeyFile {
    const MAJOR: u32 = VERSION;

    fn carry(old: &serde_json::Value, new: &mut serde_json::Value) {
        kept::carry(old, new);
        for (old, new) in kept::pairs(old, new, "/feeders", &["/identity"]) {
            kept::carry(old, new);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct EnrolledFeeder {
    /// The feeder's X25519 identity public key, hex. Its first 8 bytes are
    /// the fingerprint the feeder logs (`PLID1`).
    pub identity: String,
}

/// The key file `bytes` hold.
pub fn read(bytes: &[u8]) -> Result<OwnerKeyFile, OwnerError> {
    match kept::read(bytes)? {
        Found::Read(file) => Ok(file),
        Found::Newer { major } => Err(OwnerError::Newer {
            major,
            reads: VERSION,
        }),
    }
}

/// The key file at `path`.
pub fn load(path: &Path) -> Result<OwnerKeyFile, OwnerError> {
    read(&std::fs::read(path)?)
}

/// Replace the key file at `path` with `file`, unless a newer build's is
/// there.
pub fn save(path: &Path, file: &OwnerKeyFile) -> Result<(), OwnerError> {
    kept::write::<OwnerKeyFile>(path, file)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../tests/fixtures/owner-v1.json");

    #[test]
    fn version_1_reads_as_its_sample() {
        let file = read(SAMPLE.as_bytes()).unwrap();

        assert_eq!(
            file,
            OwnerKeyFile {
                version: 1,
                secret: "11".repeat(32),
                label: "laptop".into(),
                feeders: vec![EnrolledFeeder {
                    identity: "22".repeat(32),
                }],
            }
        );
        // No field can be absent, so the sample written back is every key
        // written (`crate::kept`).
        let written: serde_json::Value = serde_json::to_value(&file).unwrap();
        let sample: serde_json::Value = serde_json::from_str(SAMPLE).unwrap();
        assert_eq!(written, sample, "written as the sample");
    }

    #[test]
    fn every_field_is_required_and_an_added_one_is_skipped() {
        assert!(matches!(
            read(br#"{"version":1,"secret":"00","label":"x"}"#),
            Err(OwnerError::Unreadable(kept::Unreadable::Shape(_)))
        ));
        let grown = br#"{"version":1,"secret":"00","label":"x","feeders":[],"added":1}"#;
        assert_eq!(read(grown).unwrap().label, "x");
    }

    #[test]
    fn a_newer_major_is_refused_and_never_written_over() {
        let newer = br#"{"version":2,"anything":true}"#;
        assert!(matches!(
            read(newer),
            Err(OwnerError::Newer { major: 2, reads: 1 })
        ));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("owner.json");
        std::fs::write(&path, newer).unwrap();
        let file = read(SAMPLE.as_bytes()).unwrap();
        assert!(matches!(save(&path, &file), Err(OwnerError::Write(_))));
        assert_eq!(std::fs::read(&path).unwrap(), newer);
    }

    /// A newer minor's fields, at the top and in a feeder, outlast a save.
    #[test]
    fn a_save_carries_what_a_newer_minor_added() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("owner.json");
        let mut grown: serde_json::Value = serde_json::from_str(SAMPLE).unwrap();
        grown["added"] = serde_json::json!(1);
        grown["feeders"][0]["added"] = serde_json::json!(2);
        std::fs::write(&path, grown.to_string()).unwrap();

        let mut file = load(&path).unwrap();
        file.label = "renamed".into();
        file.feeders.push(EnrolledFeeder {
            identity: "33".repeat(32),
        });
        save(&path, &file).unwrap();

        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["label"], "renamed");
        assert_eq!(saved["added"], 1);
        assert_eq!(saved["feeders"][0]["added"], 2);
        assert!(saved["feeders"][1].get("added").is_none());
    }
}
