//! The tools a killed server may have left running on this bench.
//!
//! A server killed outright cannot stop its job's children, and they no
//! longer count as the bench's, so `status` looks for them itself: every
//! process named as one of the bench's tools whose working directory has
//! the same `private/` as this checkout. Every job's child runs from its
//! server's checkout, and the lease lives in that `private/`, so a worktree
//! whose `private/` is a link to the main checkout's (and shares its lease)
//! finds the main server's tools too. A checkout with a `private/` of its
//! own has a bench of its own, and its tools are left alone. Nothing needs
//! recording by the server that was killed.

use std::path::Path;

/// The directory a bench's lease, and so the bench, is identified by.
const PRIVATE: &str = "private";

/// The tools a job runs, by their process names.
const TOOLS: [&str; 4] = [
    "wellfed-flash",
    "wellfed-stock",
    "wellfed-build",
    "sigrok-cli",
];
pub(crate) const FLASH: &str = "wellfed-flash";

/// One such process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Orphan {
    pub pid: u32,
    pub name: String,
    pub command: String,
}

/// Why `status --break-lease` would not free the bench.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OrphanError {
    /// It may be mid-write; breaking the lease would let a new server open
    /// the UART under it.
    #[error(
        "wellfed-flash pid {pid} (`{command}`) still runs on this bench; let it finish, or \
         stop it with SIGINT (`kill -INT {pid}`) so it records its stop, then break the lease"
    )]
    FlashAlive { pid: u32, command: String },
}

/// The bench's tools running from any checkout that shares `root`'s
/// `private/`, by pid.
pub(crate) fn running(root: &Path) -> Vec<Orphan> {
    let Ok(private) = root.join(PRIVATE).canonicalize() else {
        return Vec::new();
    };
    let mut found = processes(&private);
    found.sort_by_key(|orphan| orphan.pid);
    found
}

#[cfg(not(target_os = "macos"))]
fn processes(private: &Path) -> Vec<Orphan> {
    use std::fs;

    fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
            let process = entry.path();
            let name = fs::read_to_string(process.join("comm")).ok()?;
            let name = name.trim();
            if !TOOLS.contains(&name) {
                return None;
            }
            let cwd = fs::read_link(process.join("cwd")).ok()?;
            if cwd.join(PRIVATE).canonicalize().ok()? != private {
                return None;
            }
            let command = fs::read(process.join("cmdline")).ok()?;
            let command = command
                .split(|byte| *byte == 0)
                .filter(|word| !word.is_empty())
                .map(String::from_utf8_lossy)
                .collect::<Vec<_>>()
                .join(" ");
            Some(Orphan {
                pid,
                name: name.to_string(),
                command,
            })
        })
        .collect()
}

/// macOS has no /proc: `ps` lists the processes and `lsof` knows their
/// working directories. Both by absolute path, so nothing on `PATH` can hide
/// a running flash client.
#[cfg(target_os = "macos")]
fn processes(private: &Path) -> Vec<Orphan> {
    let listing = text("/bin/ps", &["-axww", "-o", "pid=,comm="]).unwrap_or_default();
    listing
        .lines()
        .filter_map(|line| {
            // `comm` is the path it was started by.
            let (pid, path) = line.trim().split_once(char::is_whitespace)?;
            let name = Path::new(path.trim()).file_name()?.to_str()?;
            if !TOOLS.contains(&name) {
                return None;
            }
            let cwd = text("/usr/sbin/lsof", &["-a", "-d", "cwd", "-Fn", "-p", pid])?;
            let cwd = cwd.lines().find_map(|line| line.strip_prefix('n'))?;
            if Path::new(cwd).join(PRIVATE).canonicalize().ok()? != private {
                return None;
            }
            let command = text("/bin/ps", &["-ww", "-o", "args=", "-p", pid])?;
            Some(Orphan {
                pid: pid.parse().ok()?,
                name: name.to_string(),
                command: command.trim().to_string(),
            })
        })
        .collect()
}

/// A tool's standard output, if it ran and succeeded.
#[cfg(target_os = "macos")]
fn text(tool: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(tool).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Refuse while a flash client runs on `root`'s bench.
pub(crate) fn no_flash(root: &Path) -> Result<(), OrphanError> {
    match running(root)
        .into_iter()
        .find(|orphan| orphan.name == FLASH)
    {
        Some(flash) => Err(OrphanError::FlashAlive {
            pid: flash.pid,
            command: flash.command,
        }),
        None => Ok(()),
    }
}
