//! The writes that may have run, kept in the app's private storage beside
//! its own file (`pending.json` next to `wellfed.json`), so the pin outlives
//! the app: a write that may have run is still resolved, by reading the
//! feeder's history, after a restart. It holds no secrets and no body: a feed keeps its portions,
//! which its `Fed` event is matched by, and any other write only its kind.
//! Nothing is ever sent again from a pin.
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
//! stay uncertain in the new one until the history settles them. Only the
//! app's UI isolate opens a router: one opened in a background isolate
//! would fence the UI's.
//!
//! Written atomically, so a crash keeps the old or the new file. Every write
//! is written down before it is sent, so a phone that stops mid-send finds
//! it pinned at its next start. The file is versioned ([`crate::stored`]).
//! One that does not read is set aside, never deleted, and the router opens
//! on a new one that holds every feed on every feeder back until the owner
//! has looked at the bowls ([`Book::set_aside`]): a lost pin is a write that
//! may have run, which is never sent again, and a feed that may have run
//! holds the next one back. A file that cannot be read at all (an IO error)
//! fails the open, and the app then sends no write at all rather than one
//! unpinned.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;
use wellfed_formats::kept;

use crate::api::error::{BridgeError, ErrorKind};
use crate::api::router::{PendingWrite, WriteKind};
use crate::api::store::{self, SetAside, StoreKind};
use crate::route::core::Book;
use crate::stored::{self, Opened, Versioned};

/// A write, pinned from its confirmation to its result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PinRecord {
    /// Its one idempotency key, from the owner's confirmation on.
    pub(crate) key: Uuid,
    /// The feeder's identity, lowercase hex.
    pub(crate) feeder: String,
    pub(crate) write: PinnedWrite,
    pub(crate) state: PinState,
    /// When the owner confirmed it, by the phone's clock (ms).
    pub(crate) sent_ms: i64,
    /// The newest event the session knew when it went, so a feed's `Fed`
    /// is one after it; `None` when the session knew none.
    pub(crate) mark: Option<u64>,
    /// Feeds this phone saw the feeder reserve, running when it went or
    /// started since: none of their ends is this one's.
    pub(crate) excluded: Vec<u32>,
    /// When the owner put it away: it no longer holds a new feed back.
    pub(crate) acknowledged_ms: Option<i64>,
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

/// What the pin keeps of the write it holds: a feed's portions, which
/// settle it from the feeder's history, and any other write's kind alone.
/// The body that went out is not kept, since nothing is ever sent again from
/// a pin, and it holds no device API type, so a change to the device API never
/// changes this file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "write", rename_all = "snake_case")]
pub(crate) enum PinnedWrite {
    Feed { portions: u8 },
    Other { kind: WriteKind },
}

impl PinnedWrite {
    pub(crate) const fn kind(self) -> WriteKind {
        match self {
            Self::Feed { .. } => WriteKind::Feed,
            Self::Other { kind } => kind,
        }
    }
}

impl PinRecord {
    /// As Dart sees it.
    pub(crate) fn view(&self) -> PendingWrite {
        PendingWrite {
            key: self.key.to_string(),
            feeder: self.feeder.clone(),
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

/// A feed this phone saw the feeder reserve, until its `Fed` is read
/// or the week it is kept passes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RunningFeed {
    pub(crate) feeder: String,
    pub(crate) feed: u32,
    pub(crate) until_ms: i64,
}

/// The file's shape.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct PinFile {
    pins: Vec<PinRecord>,
    running: Vec<RunningFeed>,
    /// The file before this one did not read and was set aside: until the
    /// owner says the bowls are checked, no feed goes.
    set_aside: Option<SetAside>,
}

impl Versioned for PinFile {
    const VERSION: u32 = 1;

    /// What a newer minor added: each pin by its key, each running feed by
    /// feeder and number, and the notice while it names the same file. A
    /// pin's write is a tagged enum and is never carried into.
    fn carry(old: &Value, new: &mut Value) {
        kept::carry(old, new);
        for (old, new) in kept::pairs(old, new, "/pins", &["/key"]) {
            kept::carry(old, new);
        }
        for (old, new) in kept::pairs(old, new, "/running", &["/feeder", "/feed"]) {
            kept::carry(old, new);
        }
        store::carry_set_aside(old, new);
    }
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
    /// the file, and handed to the next one that takes it over. `None` only
    /// inside [`Drop`], which lets it go under the registry's lock.
    lock: Option<Arc<File>>,
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

        let (store, fenced) = {
            let mut open = OPEN.lock().unwrap_or_else(PoisonError::into_inner);
            let fence = Arc::new(AtomicBool::new(false));
            let (lock, fenced) = match open.iter_mut().find(|held| held.path == registered) {
                Some(held) => {
                    // Fenced under the registry's lock, so the old store
                    // never writes once the new one has read the file.
                    held.fence.store(true, Ordering::SeqCst);
                    let fenced = std::mem::replace(&mut held.fence, Arc::clone(&fence));
                    (Arc::clone(&held.lock), Some(fenced))
                }
                None => {
                    let lock = Arc::new(lock_file(path)?);
                    open.push(Held {
                        path: registered.clone(),
                        lock: Arc::clone(&lock),
                        fence: Arc::clone(&fence),
                    });
                    (lock, None)
                }
            };
            let store = Self {
                path: path.to_owned(),
                registered,
                lock: Some(lock),
                fence,
            };
            (store, fenced)
        };

        match store.load() {
            Ok(book) => Ok((store, book)),
            Err(error) => {
                // This open never read the file, so the store it took the
                // file from writes again, as if it had never been asked.
                if let Some(fenced) = fenced {
                    store.hand_back(fenced);
                }
                Err(error)
            }
        }
    }

    /// Give the file back to the store this one took it from, `fenced`.
    fn hand_back(&self, fenced: Arc<AtomicBool>) {
        let mut open = OPEN.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(held) = open
            .iter_mut()
            .find(|held| held.path == self.registered && Arc::ptr_eq(&held.fence, &self.fence))
        {
            fenced.store(false, Ordering::SeqCst);
            held.fence = fenced;
        }
    }

    /// What the file holds. One that does not read is set aside and
    /// replaced by one that holds every feed back, since a feed it held may
    /// have run.
    fn load(&self) -> Result<Book, BridgeError> {
        let file = match stored::open(&self.path, StoreKind::Pending, |set_aside| PinFile {
            set_aside: Some(set_aside.clone()),
            ..PinFile::default()
        })? {
            Opened::Absent => return Ok(Book::default()),
            Opened::Read(file) | Opened::SetAside { value: file, .. } => file,
        };
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
            set_aside: file.set_aside,
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
            set_aside: book.set_aside.clone(),
        };
        stored::write(&self.path, &file)
    }
}

impl Drop for PinStore {
    fn drop(&mut self) {
        // The file stays registered, its lock with it, while any store of
        // this process still holds the lock, whichever was dropped first:
        // a fenced store still alive keeps the lock taken, and an open that
        // found no entry would be refused by its own process. The next
        // open takes the entry over, as from a live store. This store's
        // reference goes under the registry's lock, so of stores dropped
        // at once the last always sees itself last and lets the lock go.
        let mut open = OPEN.lock().unwrap_or_else(PoisonError::into_inner);
        drop(self.lock.take());
        open.retain(|held| {
            // Only the registry's reference is left: no store holds it.
            held.path != self.registered || Arc::strong_count(&held.lock) > 1
        });
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
            write: PinnedWrite::Feed { portions: 2 },
            state: PinState::Sending,
            sent_ms: 1_000,
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
            write: PinnedWrite::Other {
                kind: WriteKind::Settings,
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
            set_aside: None,
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
                set_aside: None,
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

    /// A store dropped while the one it fenced lives on leaves the file
    /// open to the next store of this process, and the last one dropped
    /// lets the lock go.
    #[test]
    fn dropping_the_newest_store_first_never_blocks_the_next_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending.json");

        let (old, _) = PinStore::open(&path).unwrap();
        let (new, _) = PinStore::open(&path).unwrap();
        drop(new);
        let (next, _) = PinStore::open(&path).expect("a fenced store left alive blocks nothing");
        assert!(next.save(&Book::default()).is_ok());
        assert!(old.save(&Book::default()).is_err(), "still fenced");

        drop(next);
        drop(old);
        let foreign = lock_file(&path).expect("the last store let the lock go");
        drop(foreign);
    }

    /// Two stores dropped at once on two threads: one of them is last, and
    /// it lets the lock go.
    #[test]
    fn stores_dropped_at_once_let_the_lock_go() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending.json");
        for _ in 0..200 {
            let (old, _) = PinStore::open(&path).unwrap();
            let (new, _) = PinStore::open(&path).unwrap();
            let start = Arc::new(std::sync::Barrier::new(2));
            let other = Arc::clone(&start);
            let dropping = std::thread::spawn(move || {
                other.wait();
                drop(old);
            });
            start.wait();
            drop(new);
            dropping.join().unwrap();

            let foreign = lock_file(&path).expect("the last store let the lock go");
            drop(foreign);
        }
    }

    /// An open whose read fails hands the file back to the store it took
    /// it from, and a later open still takes it over.
    #[test]
    fn an_open_that_cannot_read_the_file_hands_it_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending.json");
        let (old, _) = PinStore::open(&path).unwrap();
        old.save(&Book::default()).unwrap();

        // An IO error, not a file that doesn't read: a directory.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(PinStore::open(&path).is_err());
        std::fs::remove_dir(&path).unwrap();
        assert!(
            old.save(&Book::default()).is_ok(),
            "the old store writes again"
        );

        let (next, _) = PinStore::open(&path).expect("taken over");
        assert!(old.save(&Book::default()).is_err());
        assert!(next.save(&Book::default()).is_ok());
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
    fn a_pins_file_that_does_not_read_is_set_aside_and_holds_feeds_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending.json");
        std::fs::write(&path, b"{ not json").unwrap();

        let (store, book) = PinStore::open(&path).unwrap();
        let set_aside = book
            .set_aside
            .clone()
            .expect("the open says it set the file aside");
        assert_eq!(set_aside.kind, StoreKind::Pending);
        let aside = dir.path().join("pending.json.damaged-1");
        assert_eq!(set_aside.moved_to, aside.display().to_string());
        assert_eq!(std::fs::read(&aside).unwrap(), b"{ not json", "kept whole");
        assert!(book.pins.is_empty());
        drop(store);

        // The hold is on disk before anything is sent, so a restart keeps it.
        let (_store, again) = PinStore::open(&path).unwrap();
        assert_eq!(again.set_aside, Some(set_aside));
    }

    #[test]
    fn a_pins_file_of_a_version_no_reader_takes_is_set_aside_too() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending.json");
        std::fs::write(&path, br#"{"version": 0, "data": {}}"#).unwrap();

        let (_store, book) = PinStore::open(&path).unwrap();
        let set_aside = book.set_aside.expect("set aside");
        assert!(
            set_aside.reason.contains("version 0"),
            "{}",
            set_aside.reason
        );
    }

    /// After a downgrade: a newer build's pins are refused, so nothing is
    /// sent, and the file is left for that build to read again.
    #[test]
    fn a_newer_builds_pins_file_is_refused_and_left_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending.json");
        let newer = br#"{"version": 99, "data": {"pins": "of a newer shape"}}"#;
        std::fs::write(&path, newer).unwrap();

        let Err(error) = PinStore::open(&path) else {
            panic!("a newer build's pins are refused");
        };

        assert_eq!(error.kind, ErrorKind::Store);
        assert_eq!(std::fs::read(&path).unwrap(), newer);
        let aside = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().contains("damaged"))
            .count();
        assert_eq!(aside, 0, "nothing set aside");
    }

    #[test]
    fn the_file_is_versioned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending.json");
        let (store, _) = PinStore::open(&path).unwrap();
        store.save(&Book::default()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"version\": 1"), "{text}");
    }

    #[test]
    fn a_write_other_than_a_feed_keeps_only_its_kind() {
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

    /// Every field of the file set, each once.
    fn every_field() -> PinFile {
        PinFile {
            pins: vec![
                feed(1),
                PinRecord {
                    key: Uuid::from_u128(2),
                    write: PinnedWrite::Other {
                        kind: WriteKind::Settings,
                    },
                    state: PinState::Uncertain,
                    mark: Some(41),
                    excluded: vec![3, 4],
                    acknowledged_ms: Some(2_000),
                    ..feed(2)
                },
            ],
            running: vec![RunningFeed {
                feeder: "cd".repeat(32),
                feed: 5,
                until_ms: 3_000,
            }],
            set_aside: Some(SetAside {
                kind: StoreKind::Pending,
                moved_to: "/data/pending.json.damaged-1".into(),
                reason: "not this file's shape".into(),
                at_ms: 5_000,
            }),
        }
    }

    #[test]
    fn version_1_reads_and_writes_as_its_sample() {
        let sample = include_str!("../../tests/fixtures/pending-v1.json");
        stored::assert_every_field_written::<PinFile>(sample);
        let stored::Found::Read(read) = stored::read::<PinFile>(sample.as_bytes()).unwrap() else {
            panic!("version 1 is this build's");
        };
        assert_eq!(read.pins, every_field().pins);
        assert_eq!(read.running, every_field().running);
        assert_eq!(read.set_aside, every_field().set_aside);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending.json");
        stored::write(&path, &every_field()).unwrap();
        let written: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let sample: serde_json::Value = serde_json::from_str(sample).unwrap();
        assert_eq!(written, sample);
    }

    /// A newer minor's fields, at every struct the file nests, outlast a
    /// load and a save.
    #[test]
    fn a_save_carries_what_a_newer_minor_added() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending.json");
        let mut grown: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/pending-v1.json")).unwrap();
        let places = [
            "",
            "/data",
            "/data/pins/0",
            "/data/running/0",
            "/data/set_aside",
        ];
        for place in places {
            grown.pointer_mut(place).unwrap()["added"] = serde_json::json!(place);
        }
        std::fs::write(&path, grown.to_string()).unwrap();

        let (store, book) = PinStore::open(&path).unwrap();
        store.save(&book).unwrap();

        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        for place in places {
            assert_eq!(saved.pointer(place).unwrap()["added"], place, "{place}");
        }
    }
}
