//! The writes pinned to a path, kept in the app's private storage beside its
//! own file (`pending.json` next to `wellfed.json`), so the pin outlives the
//! app: a write that may have run is resolved on the path it went on after a
//! restart too. It holds no secrets: a write a server route carries keeps
//! its body (a feed, a cancel, settings, the lock, the desiccant, the plans,
//! the calibration), a meal call write and anything else (a Wi-Fi change
//! among them) only its kind. Nothing is ever sent again from a pin.
//!
//! One router owns the file: [`PinStore::open`] refuses a second open in
//! this process and, through an exclusive lock on `pending.json.lock`, in
//! any other, so two routers can never each think a feed is not pending.
//! Written atomically, so a crash keeps the old or the new file. Every write
//! is written down before it is sent, so a phone that stops mid-send finds
//! it pinned at its next start. A file that does not load fails the router's
//! open: the app then sends no write at all rather than one unpinned.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wellfed_formats::private;
use wellfed_model::{Calibration, PLANS, Plan, SettingsPatch};

use crate::api::error::{BridgeError, ErrorKind};
use crate::api::router::{PendingWrite, RoutePath, WriteKind};
use crate::route::core::Book;

/// A write pinned to the path it went on, from its confirmation to its
/// result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PinRecord {
    /// Its one idempotency key, from the owner's confirmation on.
    pub(crate) key: Uuid,
    /// The feeder's identity, lowercase hex.
    pub(crate) feeder: String,
    /// The server's id for the feeder.
    pub(crate) server_feeder: Option<Uuid>,
    /// Whose journal a write pinned to the server is in.
    pub(crate) server: Option<ServerMark>,
    pub(crate) path: RoutePath,
    pub(crate) write: PinnedWrite,
    pub(crate) state: PinState,
    /// When the owner confirmed it, by the phone's clock (ms).
    pub(crate) sent_ms: i64,
    /// When its attempt last ended (ms).
    pub(crate) tried_ms: i64,
    /// A direct feed: the newest event the session knew when it went, so
    /// its `Fed` is one after it; `None` when the session knew none.
    pub(crate) mark: Option<u64>,
    /// Direct feeds this phone saw the feeder reserve, running when it went
    /// or started since: none of their ends is this one's.
    pub(crate) excluded: Vec<u32>,
    /// When the owner put it away: it no longer holds a new feed back.
    pub(crate) acknowledged_ms: Option<i64>,
}

/// The journal a server write went to: the phone token it went under (a
/// token belongs to one server) and the account. A journal is trusted to
/// lack a write only when it is the same one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServerMark {
    pub(crate) token: Uuid,
    pub(crate) user: Uuid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PinState {
    /// Written down before it went; nothing has come back yet. A pin
    /// still sending when the app starts was cut off by the app's own
    /// end, and is uncertain.
    Sending,
    /// It may have run.
    Uncertain,
}

/// What a pinned write asked: the body its one send carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "write", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum PinnedWrite {
    Feed {
        portions: u8,
    },
    Cancel {
        feed: u32,
    },
    Settings {
        base: u32,
        changes: SettingsPatch,
    },
    Lock {
        locked: bool,
    },
    Desiccant,
    Plans {
        base: u32,
        plans: [Option<Plan>; PLANS],
    },
    Calibration {
        base: u32,
        calibration: Calibration,
    },
    /// A meal call upload. The clip itself is not kept: nothing is ever
    /// sent again from a pin.
    CallUpload,
    CallDelete,
    CallPlay,
    /// A write no server route carries, sent on the phone's own session:
    /// only its kind is kept.
    Other {
        kind: WriteKind,
    },
}

impl PinnedWrite {
    pub(crate) const fn kind(&self) -> WriteKind {
        match self {
            Self::Feed { .. } => WriteKind::Feed,
            Self::Cancel { .. } => WriteKind::Cancel,
            Self::Settings { .. } => WriteKind::Settings,
            Self::Lock { .. } => WriteKind::Lock,
            Self::Desiccant => WriteKind::Desiccant,
            Self::Plans { .. } => WriteKind::Plans,
            Self::Calibration { .. } => WriteKind::Calibration,
            Self::CallUpload | Self::CallDelete | Self::CallPlay => WriteKind::Call,
            Self::Other { kind } => *kind,
        }
    }
}

impl PinRecord {
    /// As Dart sees it.
    pub(crate) fn view(&self) -> PendingWrite {
        PendingWrite {
            key: self.key.to_string(),
            feeder: self.feeder.clone(),
            path: self.path,
            kind: self.write.kind(),
            portions: match self.write {
                PinnedWrite::Feed { portions } => Some(portions),
                _ => None,
            },
            sent_ms: self.sent_ms,
            acknowledged: self.acknowledged_ms.is_some(),
        }
    }
}

/// A direct feed this phone saw the feeder reserve, until its `Fed` is read
/// or the week it is kept passes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunningFeed {
    pub(crate) feeder: String,
    pub(crate) feed: u32,
    pub(crate) until_ms: i64,
}

/// The file's shape.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PinFile {
    pins: Vec<PinRecord>,
    running: Vec<RunningFeed>,
}

/// The pins files open in this process, by their absolute path.
static OPEN: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// The one owner of a pins file.
#[derive(Debug)]
pub(crate) struct PinStore {
    path: PathBuf,
    /// The path as [`OPEN`] holds it.
    registered: PathBuf,
    /// Held for as long as the store is open: the exclusive lock.
    _lock: File,
}

fn in_use(path: &Path) -> BridgeError {
    BridgeError::new(
        ErrorKind::InUse,
        format!("{} is already open in another router", path.display()),
    )
}

impl PinStore {
    /// Open the pins at `path` and what they hold, none when the router has
    /// never written them. A pin the last run left sending is uncertain:
    /// that run ended before it heard.
    pub(crate) fn open(path: &Path) -> Result<(Self, Book), BridgeError> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent).map_err(store_error)?;
        let name = path
            .file_name()
            .ok_or_else(|| BridgeError::new(ErrorKind::Store, "the pins path names no file"))?;
        let registered = parent.canonicalize().map_err(store_error)?.join(name);

        {
            let mut open = OPEN.lock().unwrap_or_else(PoisonError::into_inner);
            if open.contains(&registered) {
                return Err(in_use(path));
            }
            open.push(registered.clone());
        }
        let opened = Self::locked(path, registered.clone());
        if opened.is_err() {
            unregister(&registered);
        }
        let store = opened?;
        let book = store.load()?;
        Ok((store, book))
    }

    fn locked(path: &Path, registered: PathBuf) -> Result<Self, BridgeError> {
        let mut lock_path = path.as_os_str().to_owned();
        lock_path.push(".lock");
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(store_error)?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(in_use(path)),
            Err(TryLockError::Error(error)) => return Err(store_error(error)),
        }
        Ok(Self {
            path: path.to_owned(),
            registered,
            _lock: lock,
        })
    }

    fn load(&self) -> Result<Book, BridgeError> {
        if !self.path.try_exists().map_err(store_error)? {
            return Ok(Book::default());
        }
        let file: PinFile = wellfed_formats::load(&self.path)?;
        let pins = file
            .pins
            .into_iter()
            .map(|pin| PinRecord {
                state: PinState::Uncertain,
                ..pin
            })
            .collect();
        Ok(Book {
            pins,
            running: file.running,
        })
    }

    pub(crate) fn save(&self, book: &Book) -> Result<(), BridgeError> {
        let file = PinFile {
            pins: book.pins.clone(),
            running: book.running.clone(),
        };
        private::write_json(&self.path, &file)?;
        Ok(())
    }
}

impl Drop for PinStore {
    fn drop(&mut self) {
        unregister(&self.registered);
    }
}

fn unregister(registered: &Path) {
    let mut open = OPEN.lock().unwrap_or_else(PoisonError::into_inner);
    open.retain(|held| held != registered);
}

fn store_error(error: std::io::Error) -> BridgeError {
    BridgeError::new(ErrorKind::Store, error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(key: u128) -> PinRecord {
        PinRecord {
            key: Uuid::from_u128(key),
            feeder: "ab".repeat(32),
            server_feeder: Some(Uuid::from_u128(9)),
            server: Some(ServerMark {
                token: Uuid::from_u128(7),
                user: Uuid::from_u128(8),
            }),
            path: RoutePath::Server,
            write: PinnedWrite::Feed { portions: 2 },
            state: PinState::Sending,
            sent_ms: 1_000,
            tried_ms: 1_000,
            mark: None,
            excluded: Vec::new(),
            acknowledged_ms: None,
        }
    }

    #[test]
    fn a_pin_survives_a_restart_and_one_left_sending_comes_back_uncertain() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending.json");
        let (store, book) = PinStore::open(&path).unwrap();
        assert_eq!(book, Book::default(), "no file is no pins");

        let settings = PinRecord {
            key: Uuid::from_u128(2),
            path: RoutePath::Direct,
            server: None,
            write: PinnedWrite::Settings {
                base: 4,
                changes: SettingsPatch {
                    volume: Some(30),
                    ..SettingsPatch::default()
                },
            },
            state: PinState::Uncertain,
            mark: Some(17),
            excluded: vec![3],
            acknowledged_ms: Some(5),
            ..feed(2)
        };
        let running = RunningFeed {
            feeder: "ab".repeat(32),
            feed: 4,
            until_ms: 9,
        };
        let saved = Book {
            pins: vec![feed(1), settings.clone()],
            running: vec![running.clone()],
        };
        store.save(&saved).unwrap();
        drop(store);

        let (_store, loaded) = PinStore::open(&path).unwrap();
        assert_eq!(
            loaded,
            Book {
                pins: vec![
                    PinRecord {
                        state: PinState::Uncertain,
                        ..feed(1)
                    },
                    settings
                ],
                running: vec![running],
            }
        );
    }

    #[test]
    fn a_pins_file_has_one_owner() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending.json");
        let (store, _) = PinStore::open(&path).unwrap();

        let again = PinStore::open(&dir.path().join(".").join("pending.json"));
        assert_eq!(
            again.map(|_| ()).map_err(|error| error.kind),
            Err(ErrorKind::InUse)
        );
        drop(store);
        assert!(PinStore::open(&path).is_ok(), "free once the first closes");
    }

    #[test]
    fn a_pins_file_that_does_not_load_fails_the_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending.json");
        std::fs::write(&path, b"{ not json").unwrap();
        assert!(PinStore::open(&path).is_err());
    }

    #[test]
    fn a_write_with_no_server_route_keeps_only_its_kind() {
        let wifi = PinRecord {
            write: PinnedWrite::Other {
                kind: WriteKind::Wifi,
            },
            ..feed(3)
        };
        let json = serde_json::to_string(&wifi).unwrap();
        assert!(json.contains("\"kind\":\"wifi\""), "{json}");
        assert_eq!(wifi.view().kind, WriteKind::Wifi);
        assert_eq!(wifi.view().portions, None);
    }
}
