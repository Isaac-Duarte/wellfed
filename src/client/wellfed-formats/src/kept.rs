//! The files an owner keeps from the public beta, versioned by the upgrade
//! rule (AGENTS.md "Compatibility"): the app's files and the CLI's. Each is
//! a JSON object whose `version` is its major.
//!
//! - **Grow first.** Within a major a file grows by fields an older build
//!   skips: none of these files' types denies unknown fields, and a field
//!   an older build did not write reads as absent in the newer one. A save
//!   carries the fields it did not read over from the file it replaces
//!   ([`Kept::carry`]), so an older build's save never drops a newer minor's
//!   data. An enum value an older build cannot read is a new major instead.
//! - **Every field written.** A kept type writes every field it knows, an
//!   absent one as `null`, never skipped (no `skip_serializing_if`): the
//!   carry takes a key the new file lacks for one this build didn't read,
//!   and would put back the value a save cleared. Each file's sample test
//!   writes it with every field that can be absent left absent, and checks
//!   every key is there.
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
use serde_json::Value;

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

    /// Copy into `new`, the file about to be written, what `old` (the file
    /// it replaces, at this major) holds that this build did not read, so a
    /// newer minor's fields outlast an older build's save. JSON passed
    /// through unread. The file's own top level by default; a type whose
    /// file nests structs carries each of them too, by its path, matching
    /// list elements by what identifies them ([`carry`], [`same`],
    /// [`pairs`]).
    fn carry(old: &Value, new: &mut Value) {
        carry(old, new);
    }
}

/// Copy each field of `old` that `new` lacks into `new`, when both are
/// objects: one level, for a plain struct only. A tagged enum is never
/// carried into, since another variant's fields would come with it.
pub fn carry(old: &Value, new: &mut Value) {
    let (Value::Object(old), Value::Object(new)) = (old, new) else {
        return;
    };
    for (key, value) in old {
        if !new.contains_key(key) {
            new.insert(key.clone(), value.clone());
        }
    }
}

/// Whether `old` and `new` are the same record: each of `keys` is present
/// in both, and equal.
pub fn same(old: &Value, new: &Value, keys: &[&str]) -> bool {
    keys.iter().all(|key| {
        let old = old.pointer(key);
        old.is_some() && old == new.pointer(key)
    })
}

/// The values at `pointer` in both, when both have one.
pub fn both<'a>(
    old: &'a Value,
    new: &'a mut Value,
    pointer: &str,
) -> Option<(&'a Value, &'a mut Value)> {
    Some((old.pointer(pointer)?, new.pointer_mut(pointer)?))
}

/// The elements of the lists at `pointer` that are the same record
/// ([`same`] on `keys`, JSON pointers within an element), each new one
/// with the old one it was read from.
pub fn pairs<'a>(
    old: &'a Value,
    new: &'a mut Value,
    pointer: &str,
    keys: &[&str],
) -> Vec<(&'a Value, &'a mut Value)> {
    let Some((Value::Array(old), Value::Array(new))) = both(old, new, pointer) else {
        return Vec::new();
    };
    new.iter_mut()
        .filter_map(|new| {
            let old = old.iter().find(|old| same(old, new, keys))?;
            Some((old, new))
        })
        .collect()
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
/// major's. What the file there holds at this major that `value` does not
/// is carried over ([`Kept::carry`]). A file there that does not read at
/// all is replaced: its reader set it aside first.
pub fn write<T: Kept>(path: &Path, value: &impl Serialize) -> Result<(), WriteRefused> {
    let mut new = serde_json::to_value(value).map_err(crate::Error::from)?;

    match there(path, T::MAJOR)? {
        Some(There::Newer(major)) => {
            return Err(WriteRefused::Newer {
                path: path.to_owned(),
                major,
                writes: T::MAJOR,
            });
        }
        Some(There::Same(old)) => T::carry(&old, &mut new),
        None => {}
    }
    crate::private::write_json(path, &new)?;
    Ok(())
}

/// What the file at `path` holds that a write must heed.
enum There {
    /// A newer major's.
    Newer(u32),
    /// This major's, as JSON.
    Same(Value),
}

fn there(path: &Path, writes: u32) -> Result<Option<There>, crate::Error> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let Ok(old) = serde_json::from_slice::<Value>(&bytes) else {
        return Ok(None);
    };
    let Some(major) = old
        .get("version")
        .and_then(Value::as_u64)
        .and_then(|major| u32::try_from(major).ok())
    else {
        return Ok(None);
    };
    Ok(match major.cmp(&writes) {
        Ordering::Greater => Some(There::Newer(major)),
        Ordering::Equal => Some(There::Same(old)),
        Ordering::Less => None,
    })
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

    #[test]
    fn a_save_carries_what_a_newer_minor_added() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kept.json");
        std::fs::write(
            &path,
            br#"{"version": 2, "count": 1, "added": {"by": "newer"}}"#,
        )
        .unwrap();

        let Found::Read(mut now) = read::<Now>(&std::fs::read(&path).unwrap()).unwrap() else {
            panic!("this major");
        };
        now.count = 4;
        write::<Now>(&path, &now).unwrap();

        let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["count"], 4);
        assert_eq!(saved["added"], serde_json::json!({"by": "newer"}));
    }

    #[test]
    fn records_are_matched_by_what_identifies_them() {
        let old = serde_json::json!({"list": [{"id": 1, "x": "a"}, {"id": 2, "x": "b"}]});
        let mut new = serde_json::json!({"list": [{"id": 2}, {"id": 3}]});

        for (old, new) in pairs(&old, &mut new, "/list", &["/id"]) {
            carry(old, new);
        }

        assert_eq!(
            new,
            serde_json::json!({"list": [{"id": 2, "x": "b"}, {"id": 3}]})
        );
    }
}
