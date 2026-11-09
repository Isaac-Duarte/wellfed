//! The files the app keeps in its private storage, each with a version:
//! `wellfed.json` (the feeders and the server, [`crate::api::store`]),
//! `pending.json` (the pinned writes, [`crate::route::pins`]) and
//! `alerts.json` ([`crate::alerts`]). Each is a JSON object
//! `{"version": n, "data": …}`.
//!
//! A build that changes one's shape bumps its [`Versioned::VERSION`] and keeps
//! the previous shape's reader in [`Versioned::earlier`], converting, so a
//! phone that updates keeps what it stored. Only the one before is read: a
//! build two versions on has passed through the one between on every phone
//! that updates in order, and one that skipped it sets the file aside.
//!
//! A file this build cannot read (damaged, from an older build with no reader
//! left, or from a newer one after a downgrade) is never deleted and never
//! blocks the app: [`open`] keeps its bytes under a numbered name beside it
//! (`pending.json.damaged-1`), replaces it with a fresh value, and says so,
//! for the app's notice. The copy comes first, so a crash between the steps
//! leaves the old file in place to be set aside again at the next start.

use std::path::Path;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use wellfed_formats::private;

use crate::api::error::{BridgeError, ErrorKind};
use crate::api::store::{SetAside, StoreKind};

/// Why a stored file does not read.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Unreadable {
    #[error("not this file's shape: {0}")]
    Shape(#[from] serde_json::Error),
    #[error("version {found}, where this build reads {reads}")]
    Version { found: u32, reads: u32 },
}

/// A file with a version.
pub(crate) trait Versioned: Serialize + DeserializeOwned {
    /// The version this build writes.
    const VERSION: u32;

    /// The value of a file at `version`, which is not [`Self::VERSION`]. A
    /// build that changes the shape reads the previous one here and converts;
    /// every other version is refused. None is kept yet: no build that wrote
    /// an earlier shape has shipped.
    fn earlier(version: u32, bytes: &[u8]) -> Result<Self, Unreadable> {
        let _ = bytes;
        Err(Unreadable::Version {
            found: version,
            reads: Self::VERSION,
        })
    }
}

/// What every version shares: the version alone, other fields unread.
#[derive(Deserialize)]
struct Header {
    version: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Current<T> {
    /// Read by [`Header`]; named here only so it is allowed.
    #[serde(rename = "version")]
    _version: u32,
    data: T,
}

#[derive(Serialize)]
struct Written<'a, T> {
    version: u32,
    data: &'a T,
}

/// The value `bytes` hold, at this build's version or the one before.
pub(crate) fn read<T: Versioned>(bytes: &[u8]) -> Result<T, Unreadable> {
    let Header { version } = serde_json::from_slice(bytes)?;
    if version == T::VERSION {
        let Current { data, .. } = serde_json::from_slice::<Current<T>>(bytes)?;
        Ok(data)
    } else {
        T::earlier(version, bytes)
    }
}

/// `value` at this build's version, atomically.
pub(crate) fn write<T: Versioned>(path: &Path, value: &T) -> Result<(), BridgeError> {
    let written = Written {
        version: T::VERSION,
        data: value,
    };
    private::write_json(path, &written)?;
    Ok(())
}

/// The value at `path`, strictly: `None` when nothing was ever written, and an
/// error for a file that does not read. For a file this run already opened
/// with [`open`], so one that no longer reads is not quietly emptied.
pub(crate) fn load<T: Versioned>(path: &Path) -> Result<Option<T>, BridgeError> {
    let Some(bytes) = read_bytes(path)? else {
        return Ok(None);
    };
    read(&bytes)
        .map(Some)
        .map_err(|why| BridgeError::new(ErrorKind::Store, format!("{}: {why}", path.display())))
}

/// The value at `path` as the app's start reads it: `None` when nothing was
/// ever written. A file that does not read is copied aside and replaced with
/// `fresh(&set_aside)`, and the result says where it went. A file that
/// cannot be read at all (an IO error, not its contents) is left alone and
/// fails the open: it may read at the next start.
pub(crate) fn open<T: Versioned>(
    path: &Path,
    kind: StoreKind,
    fresh: impl FnOnce(&SetAside) -> T,
) -> Result<Opened<T>, BridgeError> {
    let Some(bytes) = read_bytes(path)? else {
        return Ok(Opened::Absent);
    };
    let why = match read(&bytes) {
        Ok(value) => return Ok(Opened::Read(value)),
        Err(why) => why,
    };

    let moved_to = private::copy_aside(path, &bytes).map_err(store_error)?;
    let set_aside = SetAside {
        kind,
        moved_to: moved_to.display().to_string(),
        reason: why.to_string(),
        at_ms: jiff::Timestamp::now().as_millisecond(),
    };
    let value = fresh(&set_aside);
    write(path, &value)?;
    Ok(Opened::SetAside { value })
}

/// What [`open`] found.
#[derive(Debug)]
pub(crate) enum Opened<T> {
    /// Nothing was ever written.
    Absent,
    Read(T),
    /// The file did not read: it was kept aside, and `value`, which `fresh`
    /// made from where it went, replaced it.
    SetAside {
        value: T,
    },
}

fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>, BridgeError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(store_error(error)),
    }
}

fn store_error(error: std::io::Error) -> BridgeError {
    BridgeError::new(ErrorKind::Store, error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Now {
        count: u32,
    }

    impl Versioned for Now {
        const VERSION: u32 = 2;

        fn earlier(version: u32, bytes: &[u8]) -> Result<Self, Unreadable> {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Before {
                tally: u32,
            }
            match version {
                1 => {
                    let Current { data, .. } = serde_json::from_slice::<Current<Before>>(bytes)?;
                    Ok(Self { count: data.tally })
                }
                found => Err(Unreadable::Version {
                    found,
                    reads: Self::VERSION,
                }),
            }
        }
    }

    #[test]
    fn a_file_reads_back_at_its_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.json");
        write(&path, &Now { count: 3 }).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"version\": 2"), "{text}");
        assert_eq!(load::<Now>(&path).unwrap(), Some(Now { count: 3 }));
    }

    #[test]
    fn the_previous_version_reads_through_its_kept_reader() {
        let now = read::<Now>(br#"{"version": 1, "data": {"tally": 5}}"#).unwrap();
        assert_eq!(now, Now { count: 5 });
    }

    #[test]
    fn another_version_or_shape_does_not_read() {
        assert!(matches!(
            read::<Now>(br#"{"version": 3, "data": {"count": 1}}"#),
            Err(Unreadable::Version { found: 3, reads: 2 })
        ));
        assert!(matches!(
            read::<Now>(br#"{"count": 1}"#),
            Err(Unreadable::Shape(_))
        ));
        assert!(matches!(
            read::<Now>(br#"{"version": 2, "data": {"count": 1}, "more": 1}"#),
            Err(Unreadable::Shape(_))
        ));
    }

    #[test]
    fn a_file_that_does_not_read_is_kept_aside_and_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.json");
        std::fs::write(&path, b"{ not json").unwrap();

        let mut told = None;
        let fresh = |set_aside: &SetAside| {
            told = Some(set_aside.clone());
            Now { count: 9 }
        };
        let Opened::SetAside { value } = open(&path, StoreKind::App, fresh).unwrap() else {
            panic!("a damaged file is set aside");
        };
        let set_aside = told.unwrap();

        assert_eq!(value, Now { count: 9 });
        assert_eq!(set_aside.kind, StoreKind::App);
        let aside = dir.path().join("a.json.damaged-1");
        assert_eq!(set_aside.moved_to, aside.display().to_string());
        assert_eq!(std::fs::read(&aside).unwrap(), b"{ not json", "kept whole");
        assert_eq!(load::<Now>(&path).unwrap(), Some(Now { count: 9 }));
        assert!(matches!(
            open(&path, StoreKind::App, |_| Now::default()).unwrap(),
            Opened::Read(Now { count: 9 })
        ));
    }

    #[test]
    fn nothing_written_is_absent_and_a_strict_load_refuses_damage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.json");
        assert!(matches!(
            open(&path, StoreKind::App, |_| Now::default()).unwrap(),
            Opened::Absent
        ));
        assert_eq!(load::<Now>(&path).unwrap(), None);

        std::fs::write(&path, b"[]").unwrap();
        assert_eq!(load::<Now>(&path).unwrap_err().kind, ErrorKind::Store);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"[]",
            "a strict load moves nothing"
        );
    }
}
