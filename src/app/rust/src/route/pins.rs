//! The writes pinned to a path, kept in the app's private storage beside its
//! own file (`pending.json` next to `wellfed.json`), so the pin outlives the
//! app: a write that may have run is resolved on the path it went on after a
//! restart too. It holds no secrets: a write a server route carries keeps
//! its body (a feed, a cancel, settings, the lock, the desiccant, a meal's
//! skip, the plans, the calibration), a meal call write and anything else (a Wi-Fi change
//! among them) only its kind. Nothing is ever sent again from a pin.
//!
//! One router owns the file at a time, so two routers can never each think
//! a feed is not pending. Another process is refused by an exclusive lock on
//! `pending.json.lock`. In this process the newest open takes the file over
//! and fences the store it took it from, which then writes nothing again,
//! so it sends no write: a Dart isolate that restarted in a live process
//! (an activity recreated, a hot restart) leaves its router behind, never
//! closed, and the app must not be left sending nothing until the process
//! dies. What the fenced router had pinned is on disk, and the new one
//! reads it back as uncertain. What the fenced router learns after the
//! fence (an answer it heard, a settlement) is lost with it, and its pins
//! stay uncertain in the new one until their own path settles them. Only the
//! app's UI isolate opens a router: one opened in a background isolate
//! would fence the UI's.
//!
//! The file also keeps the adoptions whose answer was lost, so the grace
//! before a second owner invite outlives a restart.
//! Written atomically, so a crash keeps the old or the new file. Every write
//! is written down before it is sent, so a phone that stops mid-send finds
//! it pinned at its next start. A file that does not load fails the router's
//! open: the app then sends no write at all rather than one unpinned.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wellfed_formats::private;
use wellfed_model::{Calibration, FeederSettingsPatch, PLANS, Plan};

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
/// token belongs to one server) and the account. These identify the caller,
/// not a continuous history: a server backup also restores both of them.
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
        changes: FeederSettingsPatch,
    },
    Lock {
        locked: bool,
    },
    Desiccant,
    /// A skip of one of today's meals, named by its instant.
    Skip {
        slot: u8,
        due: Timestamp,
    },
    Unskip {
        slot: u8,
        due: Timestamp,
    },
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
            Self::Skip { .. } | Self::Unskip { .. } => WriteKind::Skip,
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

/// An adoption of `feeder` whose answer was lost at `at_ms`: until the
/// server lists it, or the router's grace passes, no second owner invite is
/// made for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LostAdoption {
    pub(crate) feeder: String,
    pub(crate) at_ms: i64,
}

/// The file's shape.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PinFile {
    pins: Vec<PinRecord>,
    running: Vec<RunningFeed>,
    adoptions: Vec<LostAdoption>,
}

/// A pins file open in this process: its lock, and the fence of the store
/// that holds it now.
struct Held {
    path: PathBuf,
    lock: Arc<File>,
    fence: Arc<AtomicBool>,
}

/// The pins files open in this process, by their absolute path.
static OPEN: Mutex<Vec<Held>> = Mutex::new(Vec::new());

/// The one owner of a pins file.
#[derive(Debug)]
pub(crate) struct PinStore {
    path: PathBuf,
    /// The path as [`OPEN`] holds it.
    registered: PathBuf,
    /// The exclusive lock, held for as long as a store of this process has
    /// the file, and handed to the next one that takes it over.
    _lock: Arc<File>,
    /// Set once a newer store in this process took the file over: this one
    /// writes nothing again.
    fence: Arc<AtomicBool>,
}

fn in_use(path: &Path) -> BridgeError {
    BridgeError::new(
        ErrorKind::InUse,
        format!("{} is already open in another router", path.display()),
    )
}

fn taken_over(path: &Path) -> BridgeError {
    BridgeError::new(
        ErrorKind::InUse,
        format!("a newer router took {} over", path.display()),
    )
}

impl PinStore {
    /// Open the pins at `path` and what they hold, none when the router has
    /// never written them. A pin the last run left sending is uncertain:
    /// that run ended before it heard, or the store taken over here still
    /// waits for its answer, which it can no longer write down.
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

        let store = {
            let mut open = OPEN.lock().unwrap_or_else(PoisonError::into_inner);
            let fence = Arc::new(AtomicBool::new(false));
            let lock = match open.iter_mut().find(|held| held.path == registered) {
                Some(held) => {
                    // Fenced under the registry's lock, so the old store
                    // never writes once the new one has read the file.
                    held.fence.store(true, Ordering::SeqCst);
                    held.fence = Arc::clone(&fence);
                    Arc::clone(&held.lock)
                }
                None => {
                    let lock = Arc::new(lock_file(path)?);
                    open.push(Held {
                        path: registered.clone(),
                        lock: Arc::clone(&lock),
                        fence: Arc::clone(&fence),
                    });
                    lock
                }
            };
            Self {
                path: path.to_owned(),
                registered,
                _lock: lock,
                fence,
            }
        };
        let book = store.load()?;
        Ok((store, book))
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
            adoptions: file.adoptions,
        })
    }

    pub(crate) fn save(&self, book: &Book) -> Result<(), BridgeError> {
        // The registry's lock orders this against a takeover's read.
        let _open = OPEN.lock().unwrap_or_else(PoisonError::into_inner);
        if self.fence.load(Ordering::SeqCst) {
            return Err(taken_over(&self.path));
        }
        let file = PinFile {
            pins: book.pins.clone(),
            running: book.running.clone(),
            adoptions: book.adoptions.clone(),
        };
        private::write_json(&self.path, &file)?;
        Ok(())
    }
}

impl Drop for PinStore {
    fn drop(&mut self) {
        // A store that was taken over leaves the file to the one that took it.
        let mut open = OPEN.lock().unwrap_or_else(PoisonError::into_inner);
        open.retain(|held| held.path != self.registered || !Arc::ptr_eq(&held.fence, &self.fence));
    }
}

/// The exclusive lock on `path`'s lock file: another process's open fails.
fn lock_file(path: &Path) -> Result<File, BridgeError> {
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(store_error)?;
    match lock.try_lock() {
        Ok(()) => Ok(lock),
        Err(TryLockError::WouldBlock) => Err(in_use(path)),
        Err(TryLockError::Error(error)) => Err(store_error(error)),
    }
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
                changes: FeederSettingsPatch {
                    volume: Some(30),
                    ..FeederSettingsPatch::default()
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
        let lost = LostAdoption {
            feeder: "cd".repeat(32),
            at_ms: 11,
        };
        let saved = Book {
            pins: vec![feed(1), settings.clone()],
            running: vec![running.clone()],
            adoptions: vec![lost.clone()],
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
                adoptions: vec![lost],
            }
        );
    }

    #[test]
    fn a_newer_store_takes_the_file_over_and_the_older_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending.json");
        let (old, _) = PinStore::open(&path).unwrap();
        let mut book = Book::default();
        book.pins.push(feed(1));
        old.save(&book).unwrap();

        // A restarted isolate's router, the old one never closed.
        let (new, loaded) = PinStore::open(&dir.path().join(".").join("pending.json")).unwrap();
        assert_eq!(loaded.pins.len(), 1, "what the old one pinned comes back");
        assert_eq!(
            old.save(&Book::default()).map_err(|error| error.kind),
            Err(ErrorKind::InUse),
            "the old store is fenced"
        );
        new.save(&loaded).unwrap();

        // Dropping the old one leaves the file to the new one.
        drop(old);
        new.save(&loaded).unwrap();
        drop(new);
        assert!(PinStore::open(&path).is_ok(), "free once the last closes");
    }

    #[test]
    fn another_process_holding_the_lock_refuses_the_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending.json");
        // Another process's lock: a description of its own, not this
        // process's registry.
        let foreign = lock_file(&path).unwrap();

        assert_eq!(
            PinStore::open(&path)
                .map(|_| ())
                .map_err(|error| error.kind),
            Err(ErrorKind::InUse)
        );
        drop(foreign);
        assert!(PinStore::open(&path).is_ok());
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
