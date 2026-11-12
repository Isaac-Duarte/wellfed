//! The release cache: a directory in the server's data directory holding,
//! for each release, its file (`<id>.plota`) and what the server knows of
//! it (`<id>.json`), the id being the manifest's digest in hex.
//!
//! A file is verified before anything is written, and each write lands
//! whole: it goes to a `.part` file, is synced, then renamed into place,
//! the release file before its record. A record therefore always has its
//! file; a crash between the two leaves a file with no record, which the
//! next start removes along with any `.part`. The directory itself is the
//! index: a household holds a handful of releases, so listing reads every
//! record.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use jiff::Timestamp;
use serde::de::IntoDeserializer as _;
use serde::de::value::StrDeserializer;
use serde::{Deserialize, Serialize};
use wellfed_api::{Release, ReleaseSigner, ReleaseSource};
use wellfed_client::release::{self, KeyId, ReleaseError, Signer};
use wellfed_model::Hex;

use crate::{Added, Error, Keys};

const FILE: &str = "plota";
const RECORD: &str = "json";
const PART: &str = "part";

/// Where a file came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    Upload,
    Github { tag: String, asset: String, id: u64 },
}

/// A release's record beside its file.
#[derive(Debug, Serialize, Deserialize)]
struct Entry {
    release: Release,
    /// The GitHub asset it was fetched as, so a check does not fetch it
    /// again.
    asset: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct Cache {
    dir: PathBuf,
    keys: Arc<Keys>,
    /// One writer at a time, so two uploads of one release cannot both
    /// find it missing and race on its files.
    writing: Writer,
}

impl Cache {
    pub(crate) async fn open(dir: PathBuf, keys: Keys) -> Result<Self, Error> {
        let swept = dir.clone();
        blocking(move || {
            fs::create_dir_all(&swept).map_err(cache(&swept))?;
            sweep(&swept)
        })
        .await?;

        Ok(Self {
            dir,
            keys: Arc::new(keys),
            writing: Writer::default(),
        })
    }

    /// Every release one of today's keys signed, newest version first
    /// (then by id, so pages are stable), its signer as those keys say. A
    /// release whose key the owner has since removed stays on disk but is
    /// not listed, as an install would refuse it.
    ///
    /// Each comes with whether it is listed, so a page cursor that names a
    /// release hidden since still marks its place.
    ///
    /// The listing reads the records without hashing every file: a record
    /// changed on disk shows as it says until the release is uploaded or
    /// fetched again, which writes it back. Nothing installs from it; an
    /// install reads the release through [`Self::file`].
    pub(crate) async fn list(&self) -> Result<Vec<(Release, bool)>, Error> {
        let dir = self.dir.clone();
        let project: Vec<KeyId> = self.keys.project.iter().map(release::key_id).collect();
        let owner: Vec<KeyId> = self.keys.owner.iter().map(release::key_id).collect();
        let mut releases: Vec<(Release, bool)> = blocking(move || entries(&dir))
            .await?
            .into_iter()
            .map(|entry| {
                let mut release = entry.release;
                let signer = if project.contains(&release.key_id.0) {
                    Some(ReleaseSigner::Project)
                } else if owner.contains(&release.key_id.0) {
                    Some(ReleaseSigner::Owner)
                } else {
                    None
                };
                if let Some(signer) = signer {
                    release.signer = signer;
                }
                (release, signer.is_some())
            })
            .collect();
        releases.sort_by(|(a, _), (b, _)| b.version.cmp(&a.version).then(a.id.0.cmp(&b.id.0)));
        Ok(releases)
    }

    /// The GitHub assets the cache holds intact. One whose file was
    /// damaged on disk is not held, so the next check fetches it again and
    /// [`Self::put`] writes it back.
    pub(crate) async fn assets(&self) -> Result<HashSet<u64>, Error> {
        let dir = self.dir.clone();
        let keys = Arc::clone(&self.keys);
        blocking(move || {
            let mut held = HashSet::new();
            for entry in entries(&dir)? {
                let Some(asset) = entry.asset else { continue };
                let id = entry.release.id;
                let file_path = path(&dir, id, FILE);
                let intact = match fs::read(&file_path) {
                    Ok(file) => !damaged(&file, id, &keys),
                    Err(error) => {
                        tracing::warn!("reading {}: {error}", file_path.display());
                        false
                    }
                };
                if intact {
                    held.insert(asset);
                } else {
                    tracing::warn!(
                        "{} is damaged; the next check fetches it again",
                        file_path.display()
                    );
                }
            }
            Ok(held)
        })
        .await
    }

    /// Verify `file` and keep it, unless the cache holds it already. A
    /// held release whose file was damaged on disk is written back from
    /// `file`, and a record that no longer says what the manifest does is
    /// written again, keeping where and when the release first came.
    pub(crate) async fn put(
        &self,
        file: Vec<u8>,
        origin: Origin,
        now: Timestamp,
    ) -> Result<(Release, Added), Error> {
        let dir = self.dir.clone();
        let keys = Arc::clone(&self.keys);

        self.writing
            .run(move || {
                let verified = verify(&file, &keys)?;
                let id = Hex(verified.digest);
                let record = path(&dir, id, RECORD);
                let file_path = path(&dir, id, FILE);

                // A manifest's digest covers its signature and names the
                // image's length and SHA-256, so every intact copy of a
                // release is these same bytes.
                if let Some(entry) = readable(&record, &file_path) {
                    let release = verified_release(
                        &verified,
                        entry.release.source.clone(),
                        entry.release.added,
                    );
                    let kept = fs::read(&file_path).map_err(cache(&file_path))?;
                    if kept != file {
                        tracing::warn!("{} was damaged; writing it back", file_path.display());
                        write_whole(&file_path, &file)?;
                    }
                    if release != entry.release {
                        tracing::warn!(
                            "{} disagreed with its manifest; writing it again",
                            record.display()
                        );
                        write_entry(
                            &record,
                            &Entry {
                                release: release.clone(),
                                asset: entry.asset,
                            },
                        )?;
                    }
                    sync_dir(&dir)?;
                    return Ok((release, Added::Known));
                }

                let entry = Entry {
                    release: verified_release(&verified, (&origin).into(), now),
                    asset: match origin {
                        Origin::Github { id, .. } => Some(id),
                        Origin::Upload => None,
                    },
                };
                write_whole(&file_path, &file)?;
                write_entry(&record, &entry)?;
                sync_dir(&dir)?;
                Ok((entry.release, Added::New))
            })
            .await
    }

    /// Release `id` and its file, verified again. What an install acts on
    /// is what the verified manifest says: the record beside the file is
    /// the server's own note, and only where and when the release came
    /// from is read from it. A file that no longer verifies as the release
    /// its name says is [`Error::Changed`]; one whose key the owner has
    /// since removed is refused as the upload would be.
    pub(crate) async fn file(&self, id: Hex<32>) -> Result<(Release, Vec<u8>), Error> {
        let dir = self.dir.clone();
        let keys = Arc::clone(&self.keys);

        blocking(move || {
            let file_path = path(&dir, id, FILE);
            let entry = readable(&path(&dir, id, RECORD), &file_path).ok_or(Error::NotFound)?;
            let file = fs::read(&file_path).map_err(cache(&file_path))?;

            let verified = match verify_release(&file, &keys) {
                Ok(verified) if verified.digest == id.0 => verified,
                Err(ReleaseError::UnknownKey) => return Err(ReleaseError::UnknownKey.into()),
                Ok(_) | Err(_) => return Err(Error::Changed),
            };
            let release = verified_release(&verified, entry.release.source, entry.release.added);
            Ok((release, file))
        })
        .await
    }
}

/// The release `verified` names, its signer as today's keys say, with
/// where and when it came from.
fn verified_release(
    verified: &release::Verified,
    source: ReleaseSource,
    added: Timestamp,
) -> Release {
    Release {
        id: Hex(verified.digest),
        version: verified.version,
        build_id: Hex(verified.build_id),
        image_len: verified.image_len,
        image_sha256: Hex(verified.image_sha256),
        key_id: Hex(verified.key_id),
        model: verified.model.into(),
        signer: match verified.signer {
            Signer::Project => ReleaseSigner::Project,
            Signer::Owner => ReleaseSigner::Owner,
        },
        source,
        added,
    }
}

/// Whether the cached `file` named `id` is damaged: it no longer verifies
/// as that release. A key the owner has since removed is not damage; the
/// release is only hidden.
fn damaged(file: &[u8], id: Hex<32>, keys: &Keys) -> bool {
    match verify_release(file, keys) {
        Ok(verified) => verified.digest != id.0,
        Err(error) => error != ReleaseError::UnknownKey,
    }
}

fn write_entry(record: &Path, entry: &Entry) -> Result<(), Error> {
    let text = serde_json::to_vec_pretty(entry).map_err(|source| Error::Entry {
        path: record.to_owned(),
        source,
    })?;
    write_whole(record, &text)
}

/// A page cursor: a release id in hex.
pub(crate) fn id(text: &str) -> Result<Hex<32>, Error> {
    let text: StrDeserializer<'_, serde::de::value::Error> = text.into_deserializer();
    Hex::deserialize(text).map_err(|_| Error::Cursor)
}

fn verify(file: &[u8], keys: &Keys) -> Result<release::Verified, Error> {
    Ok(verify_release(file, keys)?)
}

fn verify_release(file: &[u8], keys: &Keys) -> Result<release::Verified, ReleaseError> {
    let keys = release::Keys {
        project: &keys.project,
        owner: &keys.owner,
    };
    release::verify(file, &keys)
}

/// Run `work` on the blocking pool: reading, hashing and syncing a
/// megabyte is not for the async workers.
pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| Error::Task)?
}

/// One writer at a time over a store's files, which use fixed `.part`
/// names. The lock is held by the blocking work itself, not the future
/// awaiting it: a caller that is dropped (a client that hung up) leaves
/// its write running on the blocking pool, and the next writer waits for
/// that write to end rather than for the caller.
#[derive(Debug, Default)]
pub(crate) struct Writer(Arc<tokio::sync::Mutex<()>>);

impl Writer {
    pub(crate) async fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, Error> + Send + 'static,
    ) -> Result<T, Error> {
        let writing = Arc::clone(&self.0).lock_owned().await;
        blocking(move || {
            let _writing = writing;
            work()
        })
        .await
    }
}

fn path(dir: &Path, id: Hex<32>, extension: &str) -> PathBuf {
    dir.join(format!("{id}.{extension}"))
}

pub(crate) fn cache(path: &Path) -> impl FnOnce(io::Error) -> Error + '_ {
    move |source| Error::Cache {
        path: path.to_owned(),
        source,
    }
}

/// The record at `path`, or `None` when there is none.
fn read_entry(path: &Path) -> Result<Option<Entry>, Error> {
    let text = match fs::read(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(cache(path)(source)),
    };
    serde_json::from_slice(&text)
        .map(Some)
        .map_err(|source| Error::Entry {
            path: path.to_owned(),
            source,
        })
}

/// The record at `record`, if it parses and its release file is there.
/// Anything else is logged and treated as no record, so one damaged file
/// neither hides the others nor stops the release being kept again.
fn readable(record: &Path, file: &Path) -> Option<Entry> {
    let entry = match read_entry(record) {
        Ok(entry) => entry?,
        Err(error) => {
            tracing::warn!("skipping a release record: {error}");
            return None;
        }
    };
    if !file.exists() {
        tracing::warn!("skipping {}: its release file is missing", record.display());
        return None;
    }
    Some(entry)
}

/// Every readable record in `dir`.
fn entries(dir: &Path) -> Result<Vec<Entry>, Error> {
    let mut out = Vec::new();
    for item in fs::read_dir(dir).map_err(cache(dir))? {
        let path = item.map_err(cache(dir))?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == RECORD)
            && let Some(entry) = readable(&path, &path.with_extension(FILE))
        {
            out.push(entry);
        }
    }
    Ok(out)
}

/// Remove what a cut-short write left: `.part` files, and a release file
/// whose record never landed.
fn sweep(dir: &Path) -> Result<(), Error> {
    for item in fs::read_dir(dir).map_err(cache(dir))? {
        let path = item.map_err(cache(dir))?.path();
        let extension = path.extension().and_then(|extension| extension.to_str());
        let orphan = extension == Some(FILE) && !path.with_extension(RECORD).exists();
        if extension == Some(PART) || orphan {
            tracing::warn!("removing {}, left by an interrupted write", path.display());
            fs::remove_file(&path).map_err(cache(&path))?;
        }
    }
    Ok(())
}

/// Write `bytes` to `path` so that it is either absent or whole.
pub(crate) fn write_whole(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let mut part = path.as_os_str().to_owned();
    part.push(".");
    part.push(PART);
    let part = PathBuf::from(part);

    let mut file = File::create(&part).map_err(cache(&part))?;
    file.write_all(bytes).map_err(cache(&part))?;
    file.sync_all().map_err(cache(&part))?;
    fs::rename(&part, path).map_err(cache(path))
}

pub(crate) fn sync_dir(dir: &Path) -> Result<(), Error> {
    File::open(dir)
        .and_then(|dir| dir.sync_all())
        .map_err(cache(dir))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    /// A writer whose caller is dropped mid-write keeps the lock until its
    /// write ends, so the next writer never opens the same `.part` beside
    /// it.
    #[tokio::test]
    async fn a_dropped_caller_holds_the_writer_until_its_write_ends() {
        let writer = Arc::new(Writer::default());
        let (started, has_started) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));

        let first = {
            let (writer, order) = (Arc::clone(&writer), Arc::clone(&order));
            tokio::spawn(async move {
                writer
                    .run(move || {
                        started.send(()).unwrap();
                        released.recv().unwrap();
                        order.lock().unwrap().push("first");
                        Ok(())
                    })
                    .await
            })
        };
        tokio::task::spawn_blocking(move || has_started.recv().unwrap())
            .await
            .unwrap();
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());

        let second = {
            let (writer, order) = (Arc::clone(&writer), Arc::clone(&order));
            tokio::spawn(async move {
                writer
                    .run(move || {
                        order.lock().unwrap().push("second");
                        Ok(())
                    })
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(order.lock().unwrap().is_empty(), "the second write waits");

        release.send(()).unwrap();
        second.await.unwrap().unwrap();
        assert_eq!(*order.lock().unwrap(), ["first", "second"]);
    }
}
