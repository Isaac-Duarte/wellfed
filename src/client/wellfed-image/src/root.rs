//! The repository checkout whose ignored `private/` holds every input and
//! output: reviews, reads, plans, journals and reports. An input outside it,
//! or one that only reaches it through a symlink out, is refused.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use wellfed_formats::require;

use crate::error::Result;

/// A checkout without its evidence directory, or a path outside it.
#[derive(Debug, thiserror::Error)]
pub enum RootError {
    #[error("{} has no private/ evidence directory", root.display())]
    NoPrivate { root: PathBuf },
    #[error("{} must be a regular file under private/", path.display())]
    NotPrivateFile { path: PathBuf },
    #[error("{} must be a new directory under private/", path.display())]
    NotPrivateDirectory { path: PathBuf },
    /// A private input that passed the check but could not be read.
    #[error("Cannot read {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
}

#[derive(Debug, Clone)]
pub struct Root {
    root: PathBuf,
    private: PathBuf,
}

impl Root {
    pub fn new(root: &Path) -> Result<Self> {
        let root = root.canonicalize()?;
        let private = root.join("private");
        require(private.is_dir(), || RootError::NoPrivate {
            root: root.clone(),
        })?;
        Ok(Self {
            private: private.canonicalize()?,
            root,
        })
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn join(&self, path: impl AsRef<Path>) -> PathBuf {
        self.root.join(path)
    }

    /// Whether `path` resolves inside `private/`; a missing path is judged by
    /// its nearest existing ancestor.
    pub fn is_private(&self, path: &Path) -> bool {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        };

        // Symlinks are resolved wherever the path so far exists, and `..` is
        // applied lexically past that, so a path through a link out of
        // private/ is outside it even where its end does not exist yet.
        let mut full = PathBuf::new();
        for part in absolute.components() {
            match part {
                Component::ParentDir => {
                    full.pop();
                }
                Component::CurDir => {}
                other => full.push(other),
            }
            if let Ok(resolved) = full.canonicalize() {
                full = resolved;
            }
        }
        full.starts_with(&self.private)
    }

    /// A regular private input file.
    pub fn private_file(&self, path: &Path) -> Result<Vec<u8>> {
        require(self.is_private(path) && path.is_file(), || {
            RootError::NotPrivateFile {
                path: path.to_path_buf(),
            }
        })?;
        fs::read(path).map_err(|source| {
            RootError::Read {
                path: path.to_path_buf(),
                source,
            }
            .into()
        })
    }

    /// A new private output directory.
    pub fn output_directory(&self, path: &Path) -> Result<PathBuf> {
        require(!path.is_symlink() && self.is_private(path), || {
            RootError::NotPrivateDirectory {
                path: path.to_path_buf(),
            }
        })?;

        private_directory().create(path).or_else(|error| {
            // A reader's output may sit below directories not made yet.
            if error.kind() == io::ErrorKind::NotFound {
                private_directory().recursive(true).create(path)
            } else {
                Err(error)
            }
        })?;
        Ok(std::path::absolute(path)?)
    }
}

/// A builder for a directory only its owner can open: mode 0700 on Unix. Off
/// Unix there are no modes, and a directory under the user's profile already
/// carries that user's access list.
fn private_directory() -> fs::DirBuilder {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;

        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder
    }
    #[cfg(not(unix))]
    fs::DirBuilder::new()
}
