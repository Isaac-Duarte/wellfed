//! The child processes a bench job has running, as one registry the bench
//! server reads for its live state and signals on a cancel.
//!
//! Every registered child starts in a process group of its own
//! ([`own_group`]). A terminal's Ctrl+C or hangup then reaches only the
//! server, never a `wellfed-flash` writing flash, and the server forwards a
//! cancel itself:
//! - **`wellfed-flash`** gets SIGINT, to its pid alone, and then stops its
//!   session terminally and records it. A flash session that writes is
//!   only ever sent SIGINT. A receive-only capture is still killed after it
//!   by the sequence that waits on it, as it always was.
//! - **Every other child** gets SIGTERM to its whole group, so a build's
//!   `cargo` and `make` stop with it rather than run on under the next job.

use std::os::unix::process::CommandExt;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex, MutexGuard};

use rustix::process::{Pid, Signal, kill_process, kill_process_group};
use wellfed_bench_formats::live::{Activity, ActivityKind};
use wellfed_bench_formats::now;

/// Told the whole list each time a child starts or is reaped. It runs with
/// the registry locked, so lists arrive in order; it must not call back in.
type Watcher = Box<dyn Fn(&[Activity]) + Send + Sync>;

/// Start `command` in a process group of its own, as every child the
/// registry tracks must be, and with no terminal input: a child outside the
/// terminal's foreground group would be stopped by a read of it.
pub fn own_group(command: &mut Command) -> &mut Command {
    command.process_group(0).stdin(std::process::Stdio::null())
}

/// [`own_group`], but with standard input a pipe its owner writes: only
/// for a child that takes answers there (`wellfed restore-stock --prompts
/// lines`). The pipe is no terminal, so no read of it can stop the child.
pub fn own_group_with_input(command: &mut Command) -> &mut Command {
    command.process_group(0).stdin(std::process::Stdio::piped())
}

/// Shared by every clone: a job's tools and its suite register into the
/// same one.
#[derive(Clone, Default)]
pub struct Children {
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    running: Mutex<Running>,
    watcher: Mutex<Option<Watcher>>,
}

/// The children registered and not yet reaped, and whether the job was
/// interrupted: a cancel holds for every child tracked after it too, so one
/// spawned in the instant before it registers is signalled as it does.
#[derive(Default)]
struct Running {
    children: Vec<Activity>,
    interrupted: bool,
}

impl Children {
    pub fn new() -> Self {
        Self::default()
    }

    /// Tell `watcher` of every change from now on, starting with the list
    /// as it is.
    pub fn watch(&self, watcher: impl Fn(&[Activity]) + Send + Sync + 'static) {
        let running = self.running_list();
        watcher(&running.children);
        *lock(&self.shared.watcher) = Some(Box::new(watcher));
    }

    /// The children registered and not yet reaped.
    #[cfg(test)]
    pub(crate) fn running(&self) -> Vec<Activity> {
        self.running_list().children.clone()
    }

    /// Register `child`, started through [`own_group`], until the returned
    /// guard drops. Its owner drops the guard as soon as it reaps the child.
    /// `label` is its subcommand words. After [`Self::interrupt`] the child
    /// is signalled at once, as the cancel would have.
    pub fn track(&self, child: &Child, kind: ActivityKind, label: &str) -> Tracked {
        let pid = child.id();
        let activity = Activity {
            kind,
            pid,
            label: label.to_string(),
            started: now(),
        };
        let mut running = self.running_list();
        if running.interrupted {
            signal(&activity);
        }
        running.children.push(activity);
        self.notify(&running.children);
        Tracked {
            children: self.clone(),
            pid,
        }
    }

    /// Signal every registered child as a cancel stops it, and every child
    /// registered from now on. Nothing is waited on here; each owner reaps
    /// its child through its own cleanup.
    pub fn interrupt(&self) {
        let mut running = self.running_list();
        running.interrupted = true;
        for child in &running.children {
            signal(child);
        }
    }

    fn running_list(&self) -> MutexGuard<'_, Running> {
        lock(&self.shared.running)
    }

    fn notify(&self, running: &[Activity]) {
        if let Some(watcher) = &*lock(&self.shared.watcher) {
            watcher(running);
        }
    }
}

/// A child's place in the registry, held beside the child and dropped once
/// it is reaped.
pub struct Tracked {
    children: Children,
    pid: u32,
}

impl Drop for Tracked {
    fn drop(&mut self) {
        let mut running = self.children.running_list();
        running.children.retain(|child| child.pid != self.pid);
        self.children.notify(&running.children);
    }
}

/// Signal `child` as a cancel stops its kind.
fn signal(child: &Activity) {
    // Owners drop a child's entry as soon as they reap it, so a listed pid
    // is still that child's (exited at worst, never reused) but for the
    // instant between the two.
    let Some(pid) = i32::try_from(child.pid).ok().and_then(Pid::from_raw) else {
        return;
    };
    match child.kind {
        ActivityKind::Flash(_) => {
            let _ = kill_process(pid, Signal::INT);
        }
        ActivityKind::Build | ActivityKind::Install | ActivityKind::Analyzer => {
            // The child leads its own group; the pid alone if it somehow
            // does not.
            if kill_process_group(pid, Signal::TERM).is_err() {
                let _ = kill_process(pid, Signal::TERM);
            }
        }
    }
}

/// The lists are plain data, so one a panicking holder left is still whole.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use wellfed_bench_formats::live::FlashUse;

    use super::*;

    #[test]
    fn the_watcher_sees_each_child_start_and_go() {
        let children = Children::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&seen);
        children.watch(move |list| {
            record
                .lock()
                .unwrap()
                .push(list.iter().map(|c| c.label.clone()).collect::<Vec<_>>());
        });

        let mut child = own_group(&mut Command::new("true")).spawn().unwrap();
        let tracked = children.track(&child, ActivityKind::Build, "build firstboot");
        assert_eq!(children.running()[0].pid, child.id());
        child.wait().unwrap();
        drop(tracked);

        assert_eq!(
            *seen.lock().unwrap(),
            [vec![], vec!["build firstboot".to_string()], vec![]]
        );
        assert!(children.running().is_empty());
    }

    fn until(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// A shell in its own group that records which signal reached it.
    fn trapping(dir: &Path) -> Child {
        let script = format!(
            "trap 'echo INT > {0}/got; kill $!; exit 1' INT; \
             trap 'echo TERM > {0}/got; kill $!; exit 1' TERM; \
             sleep 30 & echo ready > {0}/ready; wait $!",
            dir.display()
        );
        let child = own_group(Command::new("sh").arg("-c").arg(script))
            .spawn()
            .unwrap();
        until("the shell", || dir.join("ready").exists());
        child
    }

    #[test]
    fn a_cancel_interrupts_the_flash_client_and_terminates_the_rest() {
        let (flash_dir, build_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let children = Children::new();
        let mut flash = trapping(flash_dir.path());
        let mut build = trapping(build_dir.path());
        let _flash = children.track(
            &flash,
            ActivityKind::Flash(FlashUse::Write { counted: true }),
            "update execute",
        );
        let _build = children.track(&build, ActivityKind::Build, "build firstboot");

        children.interrupt();
        flash.wait().unwrap();
        build.wait().unwrap();

        let got = |dir: &Path| std::fs::read_to_string(dir.join("got")).unwrap();
        assert_eq!(got(flash_dir.path()).trim(), "INT");
        assert_eq!(got(build_dir.path()).trim(), "TERM");
    }

    /// A cancel holds for a child spawned before it and tracked after it:
    /// it is signalled as it registers, by its kind's rule.
    #[test]
    fn a_child_tracked_after_the_cancel_is_signalled_as_it_registers() {
        let (flash_dir, build_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let children = Children::new();
        children.interrupt();

        let mut flash = trapping(flash_dir.path());
        let mut build = trapping(build_dir.path());
        let _flash = children.track(
            &flash,
            ActivityKind::Flash(FlashUse::Write { counted: true }),
            "update execute",
        );
        let _build = children.track(&build, ActivityKind::Build, "build firstboot");
        flash.wait().unwrap();
        build.wait().unwrap();

        let got = |dir: &Path| std::fs::read_to_string(dir.join("got")).unwrap();
        assert_eq!(got(flash_dir.path()).trim(), "INT");
        assert_eq!(got(build_dir.path()).trim(), "TERM");
    }

    #[test]
    fn a_tracked_child_leads_its_own_group_so_a_terminal_signal_misses_it() {
        let mut child = own_group(Command::new("sleep").arg("5")).spawn().unwrap();

        let pid = Pid::from_child(&child);
        let group = rustix::process::getpgid(Some(pid)).unwrap();

        assert_eq!(group, pid);
        assert_ne!(group, rustix::process::getpgrp());
        child.kill().unwrap();
        child.wait().unwrap();
    }
}
