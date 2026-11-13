//! Durable private state files: configuration, ledger, leases and credentials.
//!
//! Every file is created mode 0600 on Unix. `write_json` replaces a file
//! atomically (`wellfed_formats::private`), so a crash leaves the old or
//! the new contents, never a torn ledger. `write_new_json`
//! refuses to replace anything, and `lock` gives one server process exclusive
//! ownership of a ledger.

use std::{fs, path::Path};

use crate::{Error, Result};
use serde::Serialize;

/// Atomically replace a private state file (`wellfed_formats::private`).
pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    wellfed_formats::private::write_json(path, value).map_err(|source| Error::Write {
        path: path.into(),
        source,
    })
}

/// Write a new private file; refuses to replace an existing one. The file
/// appears whole or not at all: the bytes go to a temporary file beside it
/// first, which is then moved into place by a step that never replaces
/// (`renameat2(RENAME_NOREPLACE)` on Linux and Android, a hard link elsewhere),
/// so a reader polling for it never sees it half written, and a writer killed
/// partway leaves no empty file that would refuse the next attempt.
pub fn write_new_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut text = serde_json::to_vec_pretty(value)?;
    text.push(b'\n');
    wellfed_formats::private::write_new_whole(path, &text).map_err(|source| Error::Create {
        path: path.into(),
        source,
    })
}

/// Hold an exclusive process lock beside the ledger for the controller lifetime.
pub fn lock(path: &Path) -> Result<fs::File> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::NoParent { path: path.into() })?;
    fs::create_dir_all(parent)?;
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let file = options.open(path.with_extension("lock"))?;
    file.try_lock()
        .map_err(|_| Error::LedgerHeld { path: path.into() })?;
    Ok(file)
}
