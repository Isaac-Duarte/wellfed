//! The files an owner keeps from the public beta, versioned by the upgrade
//! rule (AGENTS.md "Compatibility"): the app's files and the CLI's. Each is
//! a JSON object whose `version` is its major.
//!
//! - **Grow first.** Within a major a file grows by fields an older build
//!   skips: none of these files' types denies unknown fields, and a field
//!   an older build did not write reads as absent in the newer one. An enum
//!   value an older build cannot read, or a field it must not drop when it
//!   saves, is a new major instead.
//! - **A reader for every major since the beta.** A build that makes a new
//!   major keeps the old one's reader in [`Kept::earlier`] and converts.
//! - **Never write over newer data.** A file of a newer major reads as
//!   [`Found::Newer`], which its owner leaves as it is, and [`write`]
//!   refuses to replace one.
//! - **Set aside, never deleted.** A file that does not read is copied
//!   aside ([`crate::private::copy_aside`]) before anything replaces it.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// A kept file's type.
pub trait Kept: DeserializeOwned {
    /// The major this build writes.
    const MAJOR: u32;

    /// The value of a file at an older `major`. A build that makes a new
    /// major reads each older one here and converts; none is kept yet,
    /// since no major before the first has shipped.
    fn earlier(major: u32, bytes: &[u8]) -> Result<Self, Unreadable> {
        let _ = bytes;
        Err(Unreadable::NoReader {
            major,
            writes: Self::MAJOR,
        })
    }
}

/// Why a kept file does not read.
#[derive(Debug, thiserror::Error)]
pub enum Unreadable {
    #[error("not this file's shape: {0}")]
    Shape(#[from] serde_json::Error),
    #[error("version {major}, which no reader of this build takes (it writes {writes})")]
    NoReader { major: u32, writes: u32 },
}

/// What a kept file holds, as this build reads it.
#[derive(Debug, PartialEq, Eq)]
pub enum Found<T> {
    Read(T),
    /// A newer build's major, which this one cannot read and leaves as it is.
    Newer {
        major: u32,
    },
}

/// Why [`write`] did not write.
#[derive(Debug, thiserror::Error)]
pub enum WriteRefused {
    #[error(
        "{} was written by a newer build (version {major}, where this one writes {writes}), so it is left as it is",
        path.display()
    )]
    Newer {
        path: PathBuf,
        major: u32,
        writes: u32,
    },
    #[error("{0}")]
    Write(#[from] crate::Error),
}

/// What every major shares.
#[derive(serde::Deserialize)]
struct Header {
    version: u32,
}

/// The value `bytes` hold: at this build's major, at an older one through
/// its kept reader, or a newer one's, unread.
pub fn read<T: Kept>(bytes: &[u8]) -> Result<Found<T>, Unreadable> {
    let Header { version } = serde_json::from_slice(bytes)?;
    match version.cmp(&T::MAJOR) {
        Ordering::Equal => Ok(Found::Read(serde_json::from_slice(bytes)?)),
        Ordering::Less => T::earlier(version, bytes).map(Found::Read),
        Ordering::Greater => Ok(Found::Newer { major: version }),
    }
}

/// Replace `path` with `value`, a file at `T`'s major, atomically
/// ([`crate::private::write_json`]), unless the file there is a newer
/// major's. A file there that does not read at all is replaced: its reader
/// set it aside first.
pub fn write<T: Kept>(path: &Path, value: &impl Serialize) -> Result<(), WriteRefused> {
    if let Some(major) = newer(path, T::MAJOR)? {
        return Err(WriteRefused::Newer {
            path: path.to_owned(),
            major,
            writes: T::MAJOR,
        });
    }
    crate::private::write_json(path, value)?;
    Ok(())
}

/// The major of the file at `path` when it is newer than `writes`.
fn newer(path: &Path, writes: u32) -> Result<Option<u32>, crate::Error> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(serde_json::from_slice::<Header>(&bytes)
        .ok()
        .map(|header| header.version)
        .filter(|major| *major > writes))
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Now {
        version: u32,
        count: u32,
    }

    impl Kept for Now {
        const MAJOR: u32 = 2;

        fn earlier(major: u32, bytes: &[u8]) -> Result<Self, Unreadable> {
            #[derive(Deserialize)]
            struct Before {
                tally: u32,
            }
            match major {
                1 => {
                    let before: Before = serde_json::from_slice(bytes)?;
                    Ok(Self {
                        version: 2,
                        count: before.tally,
                    })
                }
                major => Err(Unreadable::NoReader { major, writes: 2 }),
            }
        }
    }

    #[test]
    fn this_major_reads_and_tolerates_a_field_a_newer_minor_added() {
        assert_eq!(
            read::<Now>(br#"{"version": 2, "count": 3, "added": true}"#).unwrap(),
            Found::Read(Now {
                version: 2,
                count: 3
            })
        );
    }

    #[test]
    fn an_older_major_reads_through_its_kept_reader() {
        assert_eq!(
            read::<Now>(br#"{"version": 1, "tally": 5}"#).unwrap(),
            Found::Read(Now {
                version: 2,
                count: 5
            })
        );
        assert!(matches!(
            read::<Now>(br#"{"version": 0}"#),
            Err(Unreadable::NoReader {
                major: 0,
                writes: 2
            })
        ));
    }

    #[test]
    fn a_newer_major_is_not_read_and_never_written_over() {
        let newer = br#"{"version": 3, "whatever": [1]}"#;
        assert_eq!(read::<Now>(newer).unwrap(), Found::Newer { major: 3 });

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kept.json");
        std::fs::write(&path, newer).unwrap();
        let value = Now {
            version: 2,
            count: 1,
        };
        assert!(matches!(
            write::<Now>(&path, &value),
            Err(WriteRefused::Newer {
                major: 3,
                writes: 2,
                ..
            })
        ));
        assert_eq!(std::fs::read(&path).unwrap(), newer, "left as it was");
    }

    #[test]
    fn a_write_replaces_this_major_an_older_one_or_damage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kept.json");
        let value = Now {
            version: 2,
            count: 7,
        };

        for there in [&b"{ not json"[..], br#"{"version": 1, "tally": 5}"#] {
            std::fs::write(&path, there).unwrap();
            write::<Now>(&path, &value).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            assert_eq!(read::<Now>(&bytes).unwrap(), Found::Read(value.clone()));
        }
    }
}
