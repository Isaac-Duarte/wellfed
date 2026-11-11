//! `wellfed-bench serve` losing its terminal, as the real binary on a
//! pseudo-terminal: a closed `ssh -t` or terminal window. The relay board is
//! a second pseudo-terminal speaking the board's lines, and the flash client
//! a stand-in, so nothing here reaches hardware.
//!
//! Linux only: what these prove is which threads are left and that they sit
//! idle, read from `/proc/<pid>/task`, which macOS has no counterpart for.

#![cfg(target_os = "linux")]

use std::fs::{self, File};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process};
use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
use tokio::runtime::Runtime;
use wellfed_bench::server::Client;
use wellfed_bench::server::stopping::CONFIRM_PROMPT;
use wellfed_formats::bench::live::{BenchState, CancelState, Phase, Uart};
use wellfed_relay::{FakeBoard, Link};

mod support;
use support::{BENCH, Screen, assert_ends, running};

/// A stand-in flash client: a capture runs for a second, and a write
/// reports its count, then runs until the test makes `finish`, recording any
/// signal that reaches it.
const FLASH: &str = r#"#!/bin/sh
case "$3 $4" in
"read observe")
  mkdir -p "$8"
  echo '{"status":"running"}' > "$8/report.json"
  trap 'kill $! 2>/dev/null; exit 1' INT TERM
  sleep 1 & wait $!
  echo '{"status":"complete","result":{"bytes":7}}' > "$8/report.json"
  ;;
"update execute")
  trap 'echo INT > "$2/update-signal"; exit 1' INT
  trap 'echo TERM > "$2/update-signal"; exit 1' TERM
  echo "Verified changed sectors: 3/19" >&2
  i=0
  while [ ! -e "$2/finish" ] && [ $i -lt 1200 ]; do sleep 0.05; i=$((i+1)); done
  ;;
esac
"#;

/// A pseudo-terminal in the board's place, answering as [`FakeBoard`] does,
/// at the path it returns.
fn relay_board() -> PathBuf {
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).unwrap();
    grantpt(&master).unwrap();
    unlockpt(&master).unwrap();
    let path = ptsname(&master, Vec::new()).unwrap();
    let path = PathBuf::from(path.to_str().unwrap());

    let mut master = File::from(master);
    // Powered, as a write needs the feeder to have stayed on since prepare.
    let mut board = FakeBoard::default();
    board.lock().on[1] = true;
    std::thread::spawn(move || {
        let mut line = Vec::new();
        let mut chunk = [0; 64];
        loop {
            // With no side open yet, or none any more, it reads EIO.
            let read = match master.read(&mut chunk) {
                Ok(read) => read,
                Err(error) if error.kind() == ErrorKind::Interrupted => 0,
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(5));
                    0
                }
            };
            for byte in &chunk[..read] {
                if *byte != b'\n' {
                    line.push(*byte);
                    continue;
                }
                let text = String::from_utf8_lossy(&line).trim().to_string();
                line.clear();
                let reply = board.transact(&text).unwrap();
                if master.write_all(&reply).is_err() {
                    return;
                }
            }
        }
    });
    path
}

/// How `serve` stands on its terminal, which decides whether a hangup signals
/// it: the kernel sends SIGHUP to a session's leader alone.
#[derive(Clone, Copy, PartialEq)]
enum Under {
    /// It is the session's leader, as under `ssh -t host cmd`: SIGHUP comes.
    Leading,
    /// A shell leads, and outlives the hangup: no signal comes, only the
    /// terminal goes. The shell then records how `serve` exits.
    Shell,
}

/// `serve` on a terminal in a temporary checkout, killed if a test fails with
/// it still running.
struct Serving {
    screen: Screen,
    dir: tempfile::TempDir,
    runtime: Runtime,
    client: Client,
    pid: Pid,
}

impl Serving {
    fn start(under: Under) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("private/verification")).unwrap();
        // The jobs' update target, `t`: a stock review, which moves nothing.
        fs::create_dir_all(root.join("t")).unwrap();
        fs::write(root.join("t/review.json"), r#"{"image_kind": "stock"}"#).unwrap();
        fs::write(
            root.join("private/bench.json"),
            serde_json::json!({
                "relay_port": relay_board(),
                "uart_port": "uart",
                "baseline": "b",
                "rom_report": "r",
            })
            .to_string(),
        )
        .unwrap();
        let flash = root.join("wellfed-flash");
        fs::write(&flash, FLASH).unwrap();
        fs::set_permissions(&flash, fs::Permissions::from_mode(0o755)).unwrap();

        let absent = root.join("absent");
        let serve = format!(
            "'{BENCH}' --root '{root}' serve --device plaf103 --flash-bin '{flash}' \
             --build-bin '{absent}' --wellfed-bin '{absent}' \
             --feeder-address 127.0.0.1:9 --no-clock-sync",
            root = root.display(),
            flash = flash.display(),
            absent = absent.display(),
        );
        let line = match under {
            Under::Leading => format!("exec {serve}"),
            Under::Shell => {
                format!(
                    "trap '' HUP; {serve}; echo $? > '{}/exit-code'",
                    root.display()
                )
            }
        };
        let screen = Screen::start(&line, &root);
        let runtime = Runtime::new().unwrap();
        let client = Client::new(&root);

        let deadline = Instant::now() + Duration::from_secs(20);
        let state = loop {
            if let Ok(state) = runtime.block_on(client.state()) {
                break state;
            }
            assert!(Instant::now() < deadline, "no server:\n{}", screen.text());
            std::thread::sleep(Duration::from_millis(20));
        };
        screen.until_shown("q or Ctrl+C stops the server");

        let pid = Pid::from_raw(i32::try_from(state.server.pid).unwrap()).unwrap();
        Self {
            screen,
            dir,
            runtime,
            client,
            pid,
        }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn state(&self) -> BenchState {
        self.runtime.block_on(self.client.state()).unwrap()
    }

    /// The server's threads called `console`: the screen's, then, once it
    /// has gone, also the writer that puts the lines back on the terminal.
    fn console_threads(&self) -> Vec<String> {
        let tasks = format!("/proc/{}/task", self.pid.as_raw_nonzero());
        fs::read_dir(tasks)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|task| {
                fs::read_to_string(task.path().join("comm"))
                    .is_ok_and(|comm| comm.trim_end() == "console")
            })
            .map(|task| task.file_name().display().to_string())
            .collect()
    }

    /// Each thread's name, state, syscall and where it waits, for a failure.
    fn threads(&self) -> String {
        let tasks = format!("/proc/{}/task", self.pid.as_raw_nonzero());
        let mut out = String::new();
        for task in fs::read_dir(tasks).into_iter().flatten().flatten() {
            let read = |name: &str| fs::read_to_string(task.path().join(name)).unwrap_or_default();
            let stat = read("stat");
            let state = stat.rsplit_once(')').map_or("", |(_, rest)| rest);
            out += &format!(
                "\n{} [{}] syscall {} wchan {} stat {}",
                read("comm").trim(),
                task.file_name().display(),
                read("syscall").trim(),
                read("wchan").trim(),
                state.get(..60).unwrap_or(state),
            );
        }
        out
    }

    /// The server's CPU time so far, in clock ticks.
    fn cpu_ticks(&self) -> u64 {
        let stat = fs::read_to_string(format!("/proc/{}/stat", self.pid.as_raw_nonzero())).unwrap();
        let (_, fields) = stat.rsplit_once(')').unwrap();
        // After the name: state is field 3 of proc_pid_stat, utime 14 and
        // stime 15.
        let fields: Vec<&str> = fields.split_whitespace().collect();
        fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
    }

    /// A write that will run until the test makes `finish`.
    fn start_write(&self) {
        let root = self.root();
        fs::write(root.join("private/verification/u1-plan.json"), "{}").unwrap();
        let words = [
            "flash-execute",
            "u1",
            "--source",
            "s",
            "--target",
            "t",
            "--expected-plan-sha256",
            "00",
            "--off-seconds",
            "0.05",
        ]
        .map(String::from);
        self.runtime
            .block_on(self.client.submit(root, &words))
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(20);
        while !matches!(self.state().uart, Uart::Writing { .. }) {
            assert!(Instant::now() < deadline, "no write: {:?}", self.state());
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Hang up, with the screen's thread known first, and see that thread
    /// end rather than spin on the dead terminal, and the process stay quiet.
    fn hang_up_and_assert_console_gone_and_quiet(&mut self) {
        let screen_threads = self.console_threads();
        assert!(!screen_threads.is_empty(), "{}", self.threads());

        self.screen.hang_up();

        let deadline = Instant::now() + Duration::from_secs(5);
        while self
            .console_threads()
            .iter()
            .any(|tid| screen_threads.contains(tid))
        {
            assert!(
                Instant::now() < deadline,
                "the console thread outlived its terminal: {}",
                self.threads()
            );
            std::thread::sleep(Duration::from_millis(20));
        }

        let before = self.cpu_ticks();
        std::thread::sleep(Duration::from_secs(2));
        let spent = self.cpu_ticks() - before;
        assert!(spent < 50, "{spent} ticks of CPU in 2 s after the hangup");
    }

    /// The write runs on, untouched, under a server that still answers.
    fn assert_write_untouched(&self) -> BenchState {
        let state = self.state();
        assert_eq!(state.server.phase, Phase::Serving);
        assert!(
            matches!(state.uart, Uart::Writing { .. }),
            "{:?}",
            state.uart
        );
        let job = state.job.as_ref().unwrap();
        assert!(
            job.end.is_none() && job.cancel == CancelState::None,
            "{job:?}"
        );
        assert!(!self.root().join("update-signal").exists());
        assert!(self.root().join("private/bench.lease").exists());
        state
    }

    /// It ended by itself, with the bench released.
    fn assert_released(&self, limit: Duration) {
        assert_ends(self.pid, limit, "the server after its terminal went");
        assert!(!self.root().join("private/bench.lease").exists());
        assert!(!self.root().join("update-signal").exists());
    }

    /// The exit code its shell recorded.
    fn exit_code(&self) -> String {
        let code = self.root().join("exit-code");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !code.exists() {
            assert!(Instant::now() < deadline, "its shell recorded no exit");
            std::thread::sleep(Duration::from_millis(20));
        }
        fs::read_to_string(code).unwrap().trim().to_string()
    }
}

impl Drop for Serving {
    fn drop(&mut self) {
        let _ = self.screen.child.kill();
        if running(self.pid) {
            let _ = kill_process(self.pid, Signal::KILL);
        }
    }
}

#[test]
fn with_no_write_under_way_sighup_stops_the_server_and_its_process_ends() {
    let mut serving = Serving::start(Under::Leading);

    serving.screen.hang_up();

    serving.assert_released(Duration::from_secs(15));
}

#[test]
fn sighup_during_a_write_ends_the_console_and_the_server_stops_only_after_the_write() {
    let mut serving = Serving::start(Under::Leading);
    serving.start_write();

    serving.hang_up_and_assert_console_gone_and_quiet();

    let state = serving.assert_write_untouched();
    assert!(state.server.stopping.is_some(), "{:?}", state.server);

    fs::write(serving.root().join("finish"), "").unwrap();

    serving.assert_released(Duration::from_secs(20));
}

#[test]
fn a_hangup_with_no_signal_ends_the_console_and_the_server_serves_until_sigterm() {
    let mut serving = Serving::start(Under::Shell);
    serving.start_write();

    serving.hang_up_and_assert_console_gone_and_quiet();

    let state = serving.assert_write_untouched();
    assert!(state.server.stopping.is_none(), "{:?}", state.server);

    // The write ends as it would have, and the server carries on after it.
    fs::write(serving.root().join("finish"), "").unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while serving.state().job.unwrap().end.is_none() {
        assert!(Instant::now() < deadline, "the job did not end");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(serving.state().server.phase, Phase::Serving);

    // The screen's end left nothing of its own in the job's log.
    let log = serving.root().join("private/bench-status/jobs/1-u1.log");
    let log = fs::read_to_string(log).unwrap();
    assert!(!log.contains("cursor"), "{log}");

    kill_process(serving.pid, Signal::TERM).unwrap();

    serving.assert_released(Duration::from_secs(15));
    assert_eq!(serving.exit_code(), "0");
}

#[test]
fn a_first_q_during_a_write_shows_the_prompt_and_touches_nothing() {
    let mut serving = Serving::start(Under::Leading);
    serving.start_write();

    serving.screen.press("q");

    serving.screen.until_shown(CONFIRM_PROMPT);
    let state = serving.assert_write_untouched();
    assert!(state.server.stopping.is_none(), "{:?}", state.server);

    fs::write(serving.root().join("finish"), "").unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while serving.state().job.unwrap().end.is_none() {
        assert!(Instant::now() < deadline, "the job did not end");
        std::thread::sleep(Duration::from_millis(20));
    }

    kill_process(serving.pid, Signal::TERM).unwrap();

    serving.assert_released(Duration::from_secs(15));
}
