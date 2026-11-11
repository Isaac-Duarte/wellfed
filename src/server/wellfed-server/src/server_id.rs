//! The server's id: `server.id` in the data directory, 16 lowercase hex
//! digits made at random on the first start and kept from then on.
//!
//! It is what the mDNS announcement carries, so the app and Home
//! Assistant's zeroconf recognise the same server after its address
//! changes, and it keeps the announced host name apart from the machine's
//! own. It is not a secret and is never derived from the sealing key: the
//! announcement is public on the LAN. A file that cannot be read or does
//! not hold an id (a first start cut short) is replaced by a new one, with
//! a warning, since nothing but a listener remembering the old one depends
//! on it. Two first starts at once (two containers on one volume) each
//! stage their id under a name of their own ([`crate::staged`]), and the
//! id linked into place first is the one both return. A staged copy a
//! stopped start left goes at the next start that reads the id.

use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::Deserialize as _;
use serde::de::IntoDeserializer as _;
use serde::de::value::StrDeserializer;
use wellfed_api::Hex;
use wellfed_service::Random;

use crate::staged::{self, StageError};

/// The id file, in the data directory.
pub const ID_FILE: &str = "server.id";

/// The prefix of the id's staged copies.
const STAGED_PREFIX: &str = "server.id.";

/// The server's id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServerId(Hex<8>);

impl ServerId {
    /// Its first 8 hex digits: enough to keep one household's servers'
    /// host names apart.
    pub fn short(&self) -> String {
        let mut text = self.0.to_string();
        text.truncate(8);
        text
    }

    fn parse(text: &str) -> Option<Self> {
        let hex: StrDeserializer<'_, serde::de::value::Error> = text.into_deserializer();
        Hex::<8>::deserialize(hex).ok().map(Self)
    }
}

impl fmt::Display for ServerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IdError {
    #[error("write {}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("make a server id")]
    Random(#[source] wellfed_service::RandomFailed),
}

/// The id kept in `data`, made with `random` when there is none.
pub fn server_id(data: &Path, random: &dyn Random) -> Result<ServerId, IdError> {
    let path = data.join(ID_FILE);
    let absent = match std::fs::read_to_string(&path) {
        Ok(text) => match ServerId::parse(text.trim_end()) {
            Some(id) => {
                remove_staged(data);
                return Ok(id);
            }
            None => {
                tracing::warn!("{} holds no server id; making a new one", path.display());
                false
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => {
            tracing::warn!("read {}: {error}; making a new server id", path.display());
            false
        }
    };

    let mut bytes = [0; 8];
    random.fill(&mut bytes).map_err(IdError::Random)?;
    let id = ServerId(Hex(bytes));
    // Staged, then linked into place when there was no file (a link never
    // replaces another start's), or renamed over one that held no id: a cut
    // leaves the old file or the whole new one.
    let (staged, mut file) =
        staged::create(data, STAGED_PREFIX, 0o666, random).map_err(|error| match error {
            StageError::Random(source) => IdError::Random(source),
            StageError::Create { path, source } => IdError::Write { path, source },
        })?;
    let placed = writeln!(file, "{id}")
        .and_then(|()| file.sync_all())
        .and_then(|()| {
            if absent {
                std::fs::hard_link(&staged, &path)
            } else {
                std::fs::rename(&staged, &path)
            }
        });
    // Gone already after a rename.
    let _ = std::fs::remove_file(&staged);
    match placed {
        Ok(()) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::NotFound
            ) =>
        {
            // Another start placed its id first, and perhaps cleared this
            // one's staged copy since: the id it placed is the one kept.
            let kept = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| ServerId::parse(text.trim_end()));
            return kept.ok_or(IdError::Write {
                path,
                source: error,
            });
        }
        Err(source) => return Err(IdError::Write { path, source }),
    }
    tracing::info!("this server's id is {id}");
    Ok(id)
}

/// Remove the id's staged copies left by starts stopped before they removed
/// theirs. A start still writing loses its copy and returns the id placed
/// first. The id is no secret, so a copy that stays is only clutter.
fn remove_staged(data: &Path) {
    let leftovers = staged::leftovers(data, STAGED_PREFIX).unwrap_or_else(|error| {
        tracing::warn!("list {}: {error}", data.display());
        Vec::new()
    });
    for path in leftovers {
        if let Err(error) = std::fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("remove {}: {error}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(u8);

    impl Random for Fixed {
        fn fill(&self, out: &mut [u8]) -> Result<(), wellfed_service::RandomFailed> {
            out.fill(self.0);
            Ok(())
        }
    }

    #[test]
    fn the_id_is_made_once_and_kept() {
        let data = tempfile::tempdir().unwrap();

        let first = server_id(data.path(), &Fixed(0xab)).unwrap();
        let again = server_id(data.path(), &Fixed(0x01)).unwrap();

        assert_eq!(first.to_string(), "abababababababab");
        assert_eq!(again, first, "a second start reads the kept id");
        assert_eq!(first.short(), "abababab");
        let text = std::fs::read_to_string(data.path().join(ID_FILE)).unwrap();
        assert_eq!(text, "abababababababab\n");
    }

    #[test]
    fn a_file_holding_no_id_is_replaced() {
        for held in [&b"abab"[..], &[0xff, 0xfe, 0x00][..]] {
            let data = tempfile::tempdir().unwrap();
            std::fs::write(data.path().join(ID_FILE), held).unwrap();

            let id = server_id(data.path(), &Fixed(0x02)).unwrap();

            assert_eq!(id.to_string(), "0202020202020202", "{held:?}");
            assert_eq!(server_id(data.path(), &Fixed(0x03)).unwrap(), id);
        }
    }

    /// A start stopped before it removed its staged copy leaves it; the next
    /// start that reads the id removes it, and nothing else.
    #[test]
    fn a_staged_copy_left_behind_is_removed() {
        let data = tempfile::tempdir().unwrap();
        let id = server_id(data.path(), &Fixed(0x05)).unwrap();
        let left = data.path().join("server.id.0101010101010101.new");
        std::fs::write(&left, "0505050505050505\n").unwrap();
        let other = data.path().join("server.key.0101010101010101.new");
        std::fs::write(&other, "not the id's").unwrap();

        assert_eq!(server_id(data.path(), &Fixed(0x06)).unwrap(), id);

        assert!(!left.exists());
        assert!(other.exists(), "another file's staged copy is not the id's");
    }

    /// A path that cannot be read as a file (here a directory) is warned
    /// about, and the start goes on once a new id is written: only that
    /// write failing stops it.
    #[test]
    fn an_unreadable_id_is_never_fatal_by_itself() {
        let data = tempfile::tempdir().unwrap();
        std::fs::create_dir(data.path().join(ID_FILE)).unwrap();

        let made = server_id(data.path(), &Fixed(0x04));

        assert!(matches!(made, Err(IdError::Write { .. })), "{made:?}");
    }

    /// A start's random source: its id, then a staged name every start
    /// draws first (as two containers' PID 1 would share one), then its own.
    struct SameFirstName {
        start: u8,
        draws: std::sync::atomic::AtomicU8,
    }

    impl Random for SameFirstName {
        fn fill(&self, out: &mut [u8]) -> Result<(), wellfed_service::RandomFailed> {
            let draw = self
                .draws
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            out.fill(match draw {
                0 => self.start,
                1 => 0xaa,
                _ => self.start ^ 0x80,
            });
            out[0] = draw;
            Ok(())
        }
    }

    #[test]
    fn racing_first_starts_return_the_id_the_file_holds() {
        const STARTS: u8 = 8;

        for trial in 0..100 {
            let data = tempfile::tempdir().unwrap();
            let barrier = std::sync::Barrier::new(usize::from(STARTS));

            let ids: Vec<_> = std::thread::scope(|scope| {
                let starts: Vec<_> = (1..=STARTS)
                    .map(|start| {
                        let (data, barrier) = (data.path(), &barrier);
                        scope.spawn(move || {
                            let random = SameFirstName {
                                start,
                                draws: std::sync::atomic::AtomicU8::new(0),
                            };
                            barrier.wait();
                            server_id(data, &random).unwrap()
                        })
                    })
                    .collect();
                starts
                    .into_iter()
                    .map(|start| start.join().unwrap())
                    .collect()
            });

            let kept = std::fs::read_to_string(data.path().join(ID_FILE)).unwrap();
            for id in ids {
                assert_eq!(format!("{id}\n"), kept, "trial {trial}");
            }
            let left = std::fs::read_dir(data.path()).unwrap().count();
            assert_eq!(left, 1, "trial {trial}: nothing left staged");
        }
    }
}
