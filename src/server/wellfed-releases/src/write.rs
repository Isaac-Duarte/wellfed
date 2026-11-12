//! How both stores write their files: one writer at a time, on the
//! blocking pool, each file landing whole through a `.part` beside it.

use std::fs::{self, File};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::Error;

/// The suffix of a file being written, which a start sweeps away.
pub(crate) const PART: &str = "part";

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

pub(crate) fn cache(path: &Path) -> impl FnOnce(io::Error) -> Error + '_ {
    move |source| Error::Cache {
        path: path.to_owned(),
        source,
    }
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
