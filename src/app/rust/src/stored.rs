//! The files the app keeps in its private storage, each with a version:
//! `wellfed.json` (the feeders and the server, [`crate::api::store`]),
//! `pending.json` (the pinned writes, [`crate::route::pins`]) and
//! `alerts.json` ([`crate::alerts`]). Each is a JSON object
//! `{"version": n, "data": …}`, kept by the beta upgrade rule
//! ([`wellfed_formats::kept`]).
//!
//! The version is the file's major. Within it the file grows by fields an
//! older build skips, so none of these types denies unknown fields. A build
//! that changes a shape past that makes a new major and keeps the old one's
//! reader in [`Versioned::earlier`], converting: a reader for every version
//! since the beta, so a phone that skipped builds keeps what it stored.
//!
//! A file from a newer build (after a downgrade) is never read, written over
//! or set aside: [`open`] and [`load`] refuse it, so the app stays off it
//! until it is updated, and going forward again finds it as it was.
//!
//! A file this build cannot read (damaged, or a version no reader takes) is
//! never deleted and never blocks the app: [`open`] keeps its bytes under a
//! numbered name beside it (`pending.json.damaged-1`), replaces it with a
//! fresh value, and says so, for the app's notice. The copy comes first, so
//! a crash between the steps leaves the old file in place to be set aside
//! again at the next start.

use std::path::Path;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub(crate) use wellfed_formats::kept::Found;
use wellfed_formats::kept::Unreadable;
use wellfed_formats::kept::{self, Kept};
use wellfed_formats::private;

use crate::api::error::{BridgeError, ErrorKind};
use crate::api::store::{SetAside, StoreKind};

/// A file with a version.
pub(crate) trait Versioned: Serialize + DeserializeOwned {
    /// The version this build writes.
    const VERSION: u32;

    /// The value of a file at an older `version`. A build that makes a new
    /// version reads each older one here and converts; none is kept yet: no
    /// build that wrote an earlier shape has shipped.
    fn earlier(version: u32, bytes: &[u8]) -> Result<Self, Unreadable> {
        let _ = bytes;
        Err(Unreadable::NoReader {
            major: version,
            writes: Self::VERSION,
        })
    }

    /// Carry what the `data` this replaces holds that this build did not
    /// read into `new` ([`Kept::carry`]): its top level by default; a file
    /// with nested structs names each one.
    fn carry(old: &Value, new: &mut Value) {
        kept::carry(old, new);
    }
}

/// A file as this build reads it at its own version: the data, with the
/// version (read by [`kept::read`]) and any field a newer build added skipped.
#[derive(Deserialize)]
struct File<T> {
    data: T,
}

impl<T: Versioned> Kept for File<T> {
    const MAJOR: u32 = T::VERSION;

    fn earlier(major: u32, bytes: &[u8]) -> Result<Self, Unreadable> {
        T::earlier(major, bytes).map(|data| Self { data })
    }

    fn carry(old: &Value, new: &mut Value) {
        kept::carry(old, new);
        if let Some((old, new)) = kept::both(old, new, "/data") {
            T::carry(old, new);
        }
    }
}

#[derive(Serialize)]
struct Written<'a, T> {
    version: u32,
    data: &'a T,
}

/// The value `bytes` hold: at this build's version, through an older
/// version's reader, or a newer build's, unread.
pub(crate) fn read<T: Versioned>(bytes: &[u8]) -> Result<Found<T>, Unreadable> {
    Ok(match kept::read::<File<T>>(bytes)? {
        Found::Read(File { data }) => Found::Read(data),
        Found::Newer { major } => Found::Newer { major },
    })
}

/// `value` at this build's version, atomically; refused over a newer
/// build's file.
pub(crate) fn write<T: Versioned>(path: &Path, value: &T) -> Result<(), BridgeError> {
    let written = Written {
        version: T::VERSION,
        data: value,
    };
    kept::write::<File<T>>(path, &written)
        .map_err(|error| BridgeError::new(ErrorKind::Store, error))
}

/// The value at `path`, strictly: `None` when nothing was ever written, and an
/// error for a file that does not read or is a newer build's. For a file this
/// run already opened with [`open`], so one that no longer reads is not
/// quietly emptied.
pub(crate) fn load<T: Versioned>(path: &Path) -> Result<Option<T>, BridgeError> {
    let Some(bytes) = read_bytes(path)? else {
        return Ok(None);
    };
    match read(&bytes) {
        Ok(Found::Read(value)) => Ok(Some(value)),
        Ok(Found::Newer { major }) => Err(newer(path, major, T::VERSION)),
        Err(why) => Err(BridgeError::new(
            ErrorKind::Store,
            format!("{}: {why}", path.display()),
        )),
    }
}

/// The value at `path` as the app's start reads it: `None` when nothing was
/// ever written. A file that does not read is copied aside and replaced with
/// `fresh(&set_aside)`, and the result says where it went. A newer build's
/// file, and one that cannot be read at all (an IO error, not its contents),
/// is left alone and fails the open: the one reads once the app is updated,
/// the other perhaps at the next start.
pub(crate) fn open<T: Versioned>(
    path: &Path,
    kind: StoreKind,
    fresh: impl FnOnce(&SetAside) -> T,
) -> Result<Opened<T>, BridgeError> {
    let Some(bytes) = read_bytes(path)? else {
        return Ok(Opened::Absent);
    };
    let why = match read(&bytes) {
        Ok(Found::Read(value)) => return Ok(Opened::Read(value)),
        Ok(Found::Newer { major }) => return Err(newer(path, major, T::VERSION)),
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

fn newer(path: &Path, major: u32, reads: u32) -> BridgeError {
    BridgeError::new(
        ErrorKind::Store,
        format!(
            "{} is version {major}, from a newer Wellfed app (this one reads up to {reads}); \
             it is left as it is until the app is updated",
            path.display()
        ),
    )
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
    struct Now {
        count: u32,
    }

    impl Versioned for Now {
        const VERSION: u32 = 2;

        fn earlier(version: u32, bytes: &[u8]) -> Result<Self, Unreadable> {
            #[derive(Deserialize)]
            struct Before {
                tally: u32,
            }
            match version {
                1 => {
                    let File { data } = serde_json::from_slice::<File<Before>>(bytes)?;
                    Ok(Self { count: data.tally })
                }
                major => Err(Unreadable::NoReader {
                    major,
                    writes: Self::VERSION,
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
    fn an_older_version_reads_through_its_kept_reader() {
        let now = read::<Now>(br#"{"version": 1, "data": {"tally": 5}}"#).unwrap();
        assert_eq!(now, Found::Read(Now { count: 5 }));
        assert!(matches!(
            read::<Now>(br#"{"version": 0, "data": {}}"#),
            Err(Unreadable::NoReader {
                major: 0,
                writes: 2
            })
        ));
    }

    #[test]
    fn a_field_a_newer_build_added_is_skipped_and_another_shape_does_not_read() {
        assert_eq!(
            read::<Now>(br#"{"version": 2, "data": {"count": 1, "more": 1}, "also": 2}"#).unwrap(),
            Found::Read(Now { count: 1 })
        );
        assert!(matches!(
            read::<Now>(br#"{"count": 1}"#),
            Err(Unreadable::Shape(_))
        ));
    }

    /// After a downgrade: the newer build's file is refused, and neither set
    /// aside nor written over, so going forward again finds it as it was.
    #[test]
    fn a_newer_builds_file_is_left_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.json");
        let newer = br#"{"version": 3, "data": {"other": []}}"#;
        std::fs::write(&path, newer).unwrap();

        assert_eq!(read::<Now>(newer).unwrap(), Found::Newer { major: 3 });
        let opened = open(&path, StoreKind::App, |_| Now::default());
        assert_eq!(opened.unwrap_err().kind, ErrorKind::Store);
        assert_eq!(load::<Now>(&path).unwrap_err().kind, ErrorKind::Store);
        assert_eq!(
            write(&path, &Now { count: 1 }).unwrap_err().kind,
            ErrorKind::Store
        );
        assert_eq!(std::fs::read(&path).unwrap(), newer, "left as it was");
        let names = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(names, 1, "nothing set aside");
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
