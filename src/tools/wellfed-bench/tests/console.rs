//! `wellfed-bench console` as an operator runs it, on a pseudo-terminal
//! that `script` provides, against a bench server in a temporary checkout on
//! a scripted relay board. One test: the server takes descriptors 1 and 2
//! for its output pipe, which no other test in this process may share.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process};
use wellfed_bench_server::api::DuringWrite;
use wellfed_bench_server::{Client, Console, Followed, Options, Server, ToolPaths};
use wellfed_formats::bench::live::{BenchState, CancelState, Phase};
use wellfed_relay::{AtBoard, FakeBoard};

mod support;
use support::{BENCH, Screen, assert_ends};

/// A stand-in flash client whose capture runs until it is signalled.
const FLASH: &str = r#"#!/bin/sh
case "$3 $4" in
"read observe")
  mkdir -p "$8"
  echo '{"status":"running"}' > "$8/report.json"
  trap 'kill $! 2>/dev/null; exit 1' INT TERM
  sleep 120 & wait $!
  ;;
esac
"#;

/// Before this on its terminal, the shell that becomes `console` says its pid.
const PID_MARK: &str = "console-pid=";

/// `console` on a terminal of its own.
fn console(root: &Path) -> Screen {
    let line = format!(
        "echo {PID_MARK}$$; exec '{BENCH}' --root '{}' console",
        root.display()
    );
    Screen::start(&line, root)
}

/// The pid of `console` itself, which `script` only started.
fn console_pid(screen: &Screen) -> Pid {
    screen.until_shown(PID_MARK);
    let text = screen.text();
    let (_, after) = text.split_once(PID_MARK).unwrap();
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    Pid::from_raw(digits.parse().unwrap()).unwrap()
}

#[test]
fn the_console_refuses_without_a_terminal_or_server_and_q_leaves_the_job_running() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("private")).unwrap();
    fs::write(
        root.join("private/bench.json"),
        r#"{"relay_port": "relay", "uart_port": "uart", "baseline": "b", "rom_report": "r"}"#,
    )
    .unwrap();
    let flash = root.join("wellfed-flash");
    fs::write(&flash, FLASH).unwrap();
    fs::set_permissions(&flash, fs::Permissions::from_mode(0o755)).unwrap();

    // No terminal: a pipe on stdout.
    let piped = Command::new(BENCH)
        .arg("--root")
        .arg(&root)
        .arg("console")
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&piped.stderr);
    assert_eq!(piped.status.code(), Some(1), "{said}");
    assert!(said.contains("ssh -t"), "{said}");

    // A terminal, but no server.
    let mut screen = console(&root);
    assert_eq!(screen.exit(), Some(1), "{}", screen.text());
    screen.until_shown("No bench server answers");

    let options = Options {
        root: root.clone(),
        tools: ToolPaths {
            flash_bin: flash,
            build_bin: root.join("absent"),
            wellfed_bin: root.join("absent"),
        },
        console: Console::Terminal,
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let server = Server::open(options, |_| {
        Ok(Box::new(AtBoard::new(Box::new(FakeBoard::default()))))
    })
    .unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let served = runtime.spawn(server.run(async {
        let _ = stopped.await;
    }));

    let client = Client::new(&root);
    let job = runtime
        .block_on(client.submit(&root, &["capture".to_string(), "held".to_string()]))
        .unwrap();

    // It draws the job, then `q` leaves it running on the server.
    let mut screen = console(&root);
    screen.until_shown("the server keeps running");
    screen.until_shown(&format!("job {job}: capture held"));
    screen.press("q");
    assert_eq!(screen.exit(), Some(0), "{}", screen.text());

    // The job's thread sets its activity a moment after the job is accepted.
    let deadline = Instant::now() + Duration::from_secs(20);
    let state: BenchState = loop {
        let state = runtime.block_on(client.state()).unwrap();
        if !state.activity.is_empty() {
            break state;
        }
        assert!(Instant::now() < deadline, "no activity: {state:?}");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(state.server.phase, Phase::Serving);
    let held = state.job.unwrap();
    assert_eq!(held.id, job);
    assert!(held.end.is_none(), "{held:?}");
    assert_eq!(held.cancel, CancelState::None, "{held:?}");

    // A signal closes the screen and gives the terminal back.
    for signal in [Signal::TERM, Signal::INT] {
        let mut screen = console(&root);
        let pid = console_pid(&screen);
        screen.until_shown("the server keeps running");
        kill_process(pid, signal).unwrap();
        assert_eq!(screen.exit(), Some(0), "{signal:?}: {}", screen.text());
        assert!(
            screen.text().contains("\x1b[?1049l"),
            "{signal:?} left the alternate screen on: {}",
            screen.text()
        );
    }

    // A dropped `ssh -t`: the terminal goes from under the console, which
    // must not outlive it.
    let mut screen = console(&root);
    let pid = console_pid(&screen);
    screen.until_shown("the server keeps running");
    screen.hang_up();
    assert_ends(pid, Duration::from_secs(5), "with its terminal gone");

    let state = runtime.block_on(client.state()).unwrap();
    assert_eq!(state.server.phase, Phase::Serving);
    assert!(state.job.unwrap().end.is_none());

    runtime
        .block_on(client.cancel(job, DuringWrite::Refuse))
        .unwrap();
    let (_interrupts, mut none) = tokio::sync::mpsc::unbounded_channel();
    let followed = runtime
        .block_on(client.follow(job, |_| {}, &mut none))
        .unwrap();
    assert!(matches!(followed, Followed::Ended(_)), "{followed:?}");

    let _ = stop.send(());
    runtime.block_on(served).unwrap().unwrap();
    wellfed_bench_server::restore_output();
}
