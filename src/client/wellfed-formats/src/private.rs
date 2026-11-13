//! Writing the private files under `private/`: secrets, ledgers and keys.
//!
//! Every file is created mode 0600 on Unix. [`write_json`] replaces a file
//! atomically (temporary file, fsync, rename, directory fsync), so a crash
//! leaves the old or the new contents, never a torn file. [`write_new`]
//! writes evidence that must never replace anything: it refuses a path that
//! exists. New secrets come from [`random`].

use std::fs;
use std::io::Write;
use std::path::Path;

use serde::Serialize;

use crate::Result;

/// `N` bytes from the operating system's CSPRNG, for new secrets: every
/// client's static and ephemeral keys, signing seeds and request ids. It
/// goes through `getrandom`, the system's own call on each platform
/// (`getrandom(2)` on Linux and Android, `ProcessPrng` on Windows), rather
/// than a device file only some systems have. A source that fails is an
/// error ([`Error::Random`](crate::Error::Random)); nothing falls back to a
/// weaker one.
pub fn random<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes)?;
    Ok(bytes)
}

/// Refuse the secret key file at `path` if users other than its owner may
/// read or write it. Every writer here creates key files 0600, so wider
/// modes mean a copy, sync or restore widened them, and a signing seed or
/// client secret others can read is as good as theirs. A filesystem that
/// keeps no Unix modes at all (FAT or exFAT on a USB stick, NTFS), where
/// every file reads as open, is the one place the mode says nothing, and
/// there the check is skipped. Off Unix there are no modes to check.
pub fn require_unshared(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = fs::metadata(path)?.permissions().mode();
        let modes = filesystem_modes(path);
        if let Some(mode) = shared(mode, modes) {
            return Err(crate::Error::Shared {
                path: path.to_path_buf(),
                mode,
            });
        }
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Whether a filesystem records each file's Unix mode.
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Modes {
    Kept,
    /// FAT, exFAT or NTFS: every file shows the mount's own mode.
    Absent,
}

/// The permission bits others hold on a file of `mode`, if any matter.
#[cfg(unix)]
fn shared(mode: u32, modes: Modes) -> Option<u32> {
    (modes == Modes::Kept && mode & 0o077 != 0).then_some(mode & 0o777)
}

/// Whether the filesystem holding `path` keeps Unix modes. One that can't be
/// asked is taken to keep them, so the check stays on.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn filesystem_modes(path: &Path) -> Modes {
    // statfs(2) magic numbers: MSDOS (FAT, vfat), exFAT, NTFS (the old
    // driver's and ntfs3's).
    const NO_MODES: [i128; 4] = [0x4d44, 0x2011_bab0, 0x5346_544e, 0x7366_746e];

    match rustix::fs::statfs(path) {
        Ok(stat) if NO_MODES.contains(&i128::from(stat.f_type)) => Modes::Absent,
        _ => Modes::Kept,
    }
}

/// Whether the filesystem holding `path` keeps Unix modes, by its type name.
#[cfg(target_vendor = "apple")]
fn filesystem_modes(path: &Path) -> Modes {
    const NO_MODES: [&[u8]; 3] = [b"msdos", b"exfat", b"ntfs"];

    let Ok(stat) = rustix::fs::statfs(path) else {
        return Modes::Kept;
    };
    let name: Vec<u8> = stat
        .f_fstypename
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| c.to_ne_bytes()[0])
        .collect();
    if NO_MODES.contains(&name.as_slice()) {
        Modes::Absent
    } else {
        Modes::Kept
    }
}

/// Other Unix systems: no filesystem is known to lack modes.
#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "android", target_vendor = "apple"))
))]
fn filesystem_modes(_: &Path) -> Modes {
    Modes::Kept
}

/// Create `path` exclusively, write it in full and fsync it.
pub fn write_new(path: &Path, data: &[u8]) -> Result<()> {
    let mut file = create_private(path)?;
    file.write_all(data)?;
    file.sync_all()?;
    Ok(())
}

/// Create `path` whole or not at all; refuses to replace an existing file.
/// The bytes go to a temporary file beside it first, which is then moved into
/// place by a step that never replaces ([`move_new`]), so a reader never sees it
/// half written, and a writer killed partway leaves no partial file that would
/// refuse the next attempt.
///
/// The temporary file's name is drawn fresh, so one a crashed writer left
/// never refuses the next, and it is removed whenever this returns: a write
/// that failed partway leaves no partial copy of a key or a backup beside
/// the path.
pub fn write_new_whole(path: &Path, data: &[u8]) -> Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("the path names no file"))?;
    let mut temporary = name.to_owned();
    let tag = wellfed_session::hex::Lower(&random::<8>()?).to_string();
    temporary.push(format!(".{tag}.tmp"));
    let temporary = path.with_file_name(temporary);

    let mut file = create_private(&temporary)?;
    let result = (|| {
        file.write_all(data)?;
        file.sync_all()?;
        move_new(&temporary, path)?;
        sync_parent(path)?;
        Ok(())
    })();

    // A rename leaves no temporary file; a link, or a failure, does.
    let _ = fs::remove_file(&temporary);
    result
}

/// Create `path` exclusively, mode 0600 on Unix.
fn create_private(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Move `from` to `to`, failing if anything is there. Android's policy denies an
/// app hard links, so Linux and Android use `renameat2(RENAME_NOREPLACE)`. A
/// filesystem that refuses it fails the write: no other step gives both whole
/// and never-replaces, so there is nothing safe to fall back to.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn move_new(from: &Path, to: &Path) -> std::io::Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};

    renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE).map_err(Into::into)
}

/// Whether [`move_new`] leaves `from` behind, for its caller to remove.
#[cfg(any(target_os = "linux", target_os = "android"))]
const MOVE_LEAVES_SOURCE: bool = false;

/// Move `from` to `to`, failing if anything is there: elsewhere a link, which
/// never replaces. The caller removes `from`.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn move_new(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::hard_link(from, to)
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
const MOVE_LEAVES_SOURCE: bool = true;

/// The names a file set aside beside `path` takes, numbered, in order.
fn aside_names(path: &Path) -> std::io::Result<impl Iterator<Item = std::path::PathBuf>> {
    const MOST: u32 = 1000;

    let name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("the path names no file"))?
        .to_owned();
    let path = path.to_owned();
    Ok((1..=MOST).map(move |number| {
        let mut aside = name.clone();
        aside.push(format!(".damaged-{number}"));
        path.with_file_name(aside)
    }))
}

fn too_many() -> std::io::Error {
    std::io::Error::other("too many files already set aside")
}

fn sync_parent(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        sync_entries(parent)?;
    }
    Ok(())
}

/// Make the entries just created, renamed or removed in `directory` durable.
///
/// On Unix that is an fsync of the directory itself. Windows can't open a
/// directory as a file to flush it, and NTFS journals directory entries on
/// its own, so there it does nothing: every caller has already synced the
/// file's data before the entry changed, which is what a crash must not lose.
pub fn sync_entries(directory: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    fs::File::open(directory)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
}

/// Move the whole file `from` to `to`, refused if anything is at `to`
/// (`AlreadyExists`): for a file written beside its path and moved in once
/// whole, which must never replace an earlier one there.
pub fn rename_new(from: &Path, to: &Path) -> std::io::Result<()> {
    move_new(from, to)?;
    if MOVE_LEAVES_SOURCE {
        fs::remove_file(from)?;
    }
    sync_parent(to)
}

/// Rename a file an owner agreed to give up, to a numbered name beside it. It is
/// never deleted, and never replaces an earlier one set aside. The new path.
pub fn set_aside(path: &Path) -> std::io::Result<std::path::PathBuf> {
    set_aside_then(path, || {})
}

/// [`set_aside`], running `moved` once the file is under its new name: a
/// test's other writer, placing a new file at `path` then. A rename leaves
/// nothing at `path`, so only a link's old name is removed, and a file
/// another writer put there after the move stays.
fn set_aside_then(path: &Path, moved: impl FnOnce()) -> std::io::Result<std::path::PathBuf> {
    let mut moved = Some(moved);
    for aside in aside_names(path)? {
        match move_new(path, &aside) {
            Ok(()) => {
                if let Some(moved) = moved.take() {
                    moved();
                }
                if MOVE_LEAVES_SOURCE {
                    match fs::remove_file(path) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error),
                    }
                }
                sync_parent(path)?;
                return Ok(aside);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(too_many())
}

/// Keep `data`, what `path` held, under the numbered name [`set_aside`]
/// would give it, durably, and leave `path` as it is. A caller that then
/// replaces `path` with [`write_json`] is crash-safe at every step: `path`
/// holds the old contents or the new, and the old are kept either way. The
/// new path.
pub fn copy_aside(path: &Path, data: &[u8]) -> std::io::Result<std::path::PathBuf> {
    for aside in aside_names(path)? {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&aside) {
            Ok(mut file) => {
                // A copy that failed partway would read later as what the
                // file held: it goes.
                let written = file.write_all(data).and_then(|()| file.sync_all());
                if let Err(error) = written {
                    let _ = fs::remove_file(&aside);
                    return Err(error);
                }
                sync_parent(path)?;
                return Ok(aside);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(too_many())
}

/// Fsync a directory, so the entries just created in it are durable.
pub fn sync_directory(path: &Path) -> Result<()> {
    sync_entries(path)?;
    Ok(())
}

/// Atomically replace a private file, syncing data before committing it.
pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let tag = wellfed_session::hex::Lower(&random::<8>()?).to_string();
    let temp = parent.join(format!(".{tag}.tmp"));

    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        let mut file = options.open(&temp)?;
        serde_json::to_writer_pretty(&mut file, value)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        sync_entries(parent)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

#[cfg(test)]
mod tests {
    /// A key file others can read is refused, by its mode; on a filesystem
    /// without Unix modes every file reads open, and nothing is refused.
    #[cfg(unix)]
    #[test]
    fn a_key_file_others_can_read_is_refused_where_modes_are_kept() {
        use std::os::unix::fs::PermissionsExt;

        use super::{Modes, require_unshared, shared};

        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("key.json");
        std::fs::write(&key, b"{}").unwrap();
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
        require_unshared(&key).unwrap();

        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            require_unshared(&key),
            Err(crate::Error::Shared { mode: 0o644, .. })
        ));
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o620)).unwrap();
        assert!(require_unshared(&key).is_err());

        // A FAT stick shows every file as the mount's mode.
        assert_eq!(shared(0o100_755, Modes::Absent), None);
        assert_eq!(shared(0o100_644, Modes::Kept), Some(0o644));
        assert_eq!(shared(0o100_600, Modes::Kept), None);
        assert_eq!(shared(0o100_400, Modes::Kept), None);
    }

    /// Two keys' worth of bytes from the system's source: 256 bits each,
    /// so equal draws would mean the source is not random at all.
    #[test]
    fn each_draw_is_fresh() {
        let first: [u8; 32] = super::random().unwrap();
        let second: [u8; 32] = super::random().unwrap();
        assert_ne!(first, second);
        assert_ne!(first, [0; 32]);
    }

    /// A file moved in once whole lands only where nothing is: an earlier
    /// one there stays, and so does the new one, under its own name.
    #[test]
    fn a_new_name_is_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let (from, to) = (
            dir.path().join("archive.partial"),
            dir.path().join("archive"),
        );
        std::fs::write(&from, b"new").unwrap();
        std::fs::write(&to, b"old").unwrap();

        let refused = super::rename_new(&from, &to).unwrap_err();
        assert_eq!(refused.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&to).unwrap(), b"old");
        assert_eq!(std::fs::read(&from).unwrap(), b"new");

        std::fs::remove_file(&to).unwrap();
        super::rename_new(&from, &to).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"new");
        assert!(!from.exists());
    }

    #[test]
    fn setting_aside_numbers_the_file_and_replaces_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.json");

        std::fs::write(&path, "one").unwrap();
        let first = super::set_aside(&path).unwrap();
        std::fs::write(&path, "two").unwrap();
        let second = super::set_aside(&path).unwrap();

        assert!(!path.exists());
        assert_eq!(first, dir.path().join("a.json.damaged-1"));
        assert_eq!(second, dir.path().join("a.json.damaged-2"));
        assert_eq!(std::fs::read(first).unwrap(), b"one");
        assert_eq!(std::fs::read(second).unwrap(), b"two");
    }

    #[test]
    fn a_copy_aside_takes_the_next_number_and_leaves_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.json");
        std::fs::write(dir.path().join("a.json.damaged-1"), "earlier").unwrap();
        std::fs::write(&path, "bad").unwrap();

        let aside = super::copy_aside(&path, b"bad").unwrap();

        assert_eq!(aside, dir.path().join("a.json.damaged-2"));
        assert_eq!(std::fs::read(&aside).unwrap(), b"bad");
        assert_eq!(std::fs::read(&path).unwrap(), b"bad", "left in place");
        assert_eq!(
            std::fs::read(dir.path().join("a.json.damaged-1")).unwrap(),
            b"earlier"
        );
    }

    use super::*;

    #[test]
    fn a_replaced_file_is_private_and_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        write_json(&path, &[1, 2]).unwrap();
        write_json(&path, &[3]).unwrap();
        assert_eq!(crate::load::<Vec<u8>>(&path).unwrap(), [3]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1, "no temp left");
    }

    #[test]
    fn a_new_file_appears_whole_and_is_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("copy.bin");
        write_new_whole(&path, b"first").unwrap();
        assert!(write_new_whole(&path, b"second").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"first");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1, "no temp left");
    }

    /// A temporary file a killed writer left, under the name this process
    /// would once have used, refuses nothing.
    #[test]
    fn a_left_temporary_file_refuses_no_later_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bin");
        let left = dir.path().join(format!("a.bin.{}.tmp", std::process::id()));
        fs::write(&left, b"partial").unwrap();

        write_new_whole(&path, b"x").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"x");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    /// A file another writer puts at the path just after it was set aside
    /// stays: a rename leaves nothing there to remove.
    #[test]
    fn a_file_written_after_a_set_aside_stays() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.json");
        fs::write(&path, "old").unwrap();

        let aside = set_aside_then(&path, || fs::write(&path, "new").unwrap()).unwrap();

        assert_eq!(fs::read(&aside).unwrap(), b"old");
        assert_eq!(fs::read(&path).unwrap(), b"new");
    }
}
