//! The last steps of a copy of the database (`VACUUM INTO`, [`Store::backup_into`]
//! or the one taken before a migration): a copy is written under a hidden
//! partial name and only then takes its own, so a name the server or an
//! owner reads is always a whole copy that outlasts a power cut. A copy
//! holds what the database does (password hashes, sealed feeder keys), so
//! only the server's user reads it, whatever its umask made it.
//!
//! [`Store::backup_into`]: crate::Store::backup_into

use std::fmt;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// A step of [`finish`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Open,
    Protect,
    Sync,
    Rename,
    SyncDirectory,
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Open => "open",
            Self::Protect => "protect",
            Self::Sync => "sync",
            Self::Rename => "rename",
            Self::SyncDirectory => "sync the directory of",
        })
    }
}

/// Why a copy did not take its place.
#[derive(Debug, thiserror::Error)]
pub enum CopyError {
    #[error("{step} {}", path.display())]
    Io {
        step: Step,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Make the copy written at `partial` the server's user's alone, sync it,
/// move it to `copy` and sync the directory.
pub fn finish(partial: &Path, copy: &Path) -> Result<(), CopyError> {
    let failed = |step, path: &Path| {
        let path = path.to_owned();
        move |source| CopyError::Io { step, path, source }
    };

    let file = fs::File::open(partial).map_err(failed(Step::Open, partial))?;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(failed(Step::Protect, partial))?;
    file.sync_all().map_err(failed(Step::Sync, partial))?;
    fs::rename(partial, copy).map_err(failed(Step::Rename, partial))?;

    let dir = copy
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::File::open(dir)
        .and_then(|dir| dir.sync_all())
        .map_err(failed(Step::SyncDirectory, dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finished_copy_takes_its_name_readable_by_its_owner_alone() {
        let dir = tempfile::tempdir().unwrap();
        let partial = dir.path().join(".copy.db.partial");
        let copy = dir.path().join("copy.db");
        fs::write(&partial, b"whole").unwrap();
        fs::set_permissions(&partial, fs::Permissions::from_mode(0o644)).unwrap();

        finish(&partial, &copy).unwrap();

        assert!(!partial.exists());
        assert_eq!(fs::read(&copy).unwrap(), b"whole");
        let mode = fs::metadata(&copy).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_missing_partial_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let partial = dir.path().join(".gone.partial");

        let Err(CopyError::Io { step, path, .. }) = finish(&partial, &dir.path().join("gone"))
        else {
            panic!("nothing to finish");
        };
        assert_eq!((step, path), (Step::Open, partial));
    }
}
