//! A file staged beside its final name under a name only its writer uses.
//!
//! The name carries random bytes, not the process id: the packaged server
//! is PID 1 in every container, so two containers on one data volume share
//! every pid-based name. The file is created with `create_new`, never after
//! removing whatever holds the name, so a writer never deletes or rewrites
//! another's staged file between its sync and its link or rename.

use std::fs::{File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use wellfed_api::Hex;
use wellfed_service::{Random, RandomFailed};

/// Names drawn before giving up: a second clash of 64 random bits means the
/// random source repeats.
const TRIES: usize = 8;

/// The suffix of every staged name.
pub(crate) const SUFFIX: &str = ".new";

#[derive(Debug)]
pub(crate) enum StageError {
    Random(RandomFailed),
    Create {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// A new file `<dir>/<prefix><16 hex digits>.new`, created exclusively with
/// `mode` (on Unix), and its path.
pub(crate) fn create(
    dir: &Path,
    prefix: &str,
    mode: u32,
    random: &dyn Random,
) -> Result<(PathBuf, File), StageError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, mode);
    #[cfg(not(unix))]
    let _ = mode;

    let mut last = None;
    for _ in 0..TRIES {
        let mut tag = [0; 8];
        random.fill(&mut tag).map_err(StageError::Random)?;
        let path = dir.join(format!("{prefix}{}{SUFFIX}", Hex(tag)));
        match options.open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(source) if source.kind() == ErrorKind::AlreadyExists => {
                last = Some((path, source));
            }
            Err(source) => return Err(StageError::Create { path, source }),
        }
    }
    let (path, source) = last.unwrap_or_else(|| {
        (
            dir.to_owned(),
            std::io::Error::from(ErrorKind::AlreadyExists),
        )
    });
    Err(StageError::Create { path, source })
}

/// Whether `name` is one [`create`] makes with `prefix`.
pub(crate) fn is_staged(name: &str, prefix: &str) -> bool {
    name.starts_with(prefix) && name.ends_with(SUFFIX)
}
