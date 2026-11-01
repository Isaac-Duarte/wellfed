//! `wellfed-bench console` as an operator runs it, on a pseudo-terminal
//! that `script` provides, against a bench server in a temporary checkout on
//! a scripted relay board. One test: the server takes descriptors 1 and 2
//! for its output pipe, which no other test in this process may share.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use wellfed_bench_server::api::DuringWrite;
use wellfed_bench_server::{Client, Console, Followed, Options, Server, ToolPaths};
use wellfed_formats::bench::live::Phase;
use wellfed_relay::{AtBoard, FakeBoard};

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

/// `console` on a terminal of its own, with its screen collected as it comes.
struct Screen {
    child: Child,
    seen: Arc<Mutex<Vec<u8>>>,
}

impl Screen {
    fn start(root: &Path) -> Self {
        let line = format!(
            "stty cols 130 rows 46; exec '{}' --root '{}' console",
            env!("CARGO_BIN_EXE_wellfed-bench"),
            root.display()
        );
        let mut child = Command::new("script")
            .args(["-qfec", &line, "/dev/null"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("this test needs util-linux's `script`");

        let mut stdout = child.stdout.take().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let collecting = Arc::clone(&seen);
        std::thread::spawn(move || {
            let mut chunk = [0; 4096];
            while let Ok(read @ 1..) = stdout.read(&mut chunk) {
                collecting.lock().unwrap().extend_from_slice(&chunk[..read]);
            }
        });
        Self { child, seen }
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.seen.lock().unwrap()).into_owned()
    }

    /// Wait for `text` on the screen. The terminal is sent only the cells
    /// that change, with cursor moves over the blanks, so the two are
    /// compared with no escape sequences or spaces.
    fn until_shown(&self, text: &str) {
        let text = plain(text);
        let deadline = Instant::now() + Duration::from_secs(20);
        while !plain(&self.text()).contains(&text) {
            assert!(
                Instant::now() < deadline,
                "{text:?} not shown:\n{}",
                self.text()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn press(&mut self, key: &str) {
        let stdin = self.child.stdin.as_mut().unwrap();
        stdin.write_all(key.as_bytes()).unwrap();
        stdin.flush().unwrap();
    }

    /// Its exit code, which it must reach soon.
    fn exit(&mut self) -> Option<i32> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status.code();
            }
            assert!(Instant::now() < deadline, "still running:\n{}", self.text());
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn plain(raw: &str) -> String {
    let mut kept = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => {
                let _ = chars.find(char::is_ascii_alphabetic);
            }
            ' ' => {}
            c => kept.push(c),
        }
    }
    kept
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
    let piped = Command::new(env!("CARGO_BIN_EXE_wellfed-bench"))
        .arg("--root")
        .arg(&root)
        .arg("console")
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&piped.stderr);
    assert_eq!(piped.status.code(), Some(1), "{said}");
    assert!(said.contains("ssh -t"), "{said}");

    // A terminal, but no server.
    let mut screen = Screen::start(&root);
    assert_eq!(screen.exit(), Some(1), "{}", screen.text());
    screen.until_shown("No bench server answers");

    let options = Options {
        root: root.clone(),
        tools: ToolPaths {
            flash_bin: flash,
            stock_bin: root.join("absent"),
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
    let mut screen = Screen::start(&root);
    screen.until_shown("the server keeps running");
    screen.until_shown(&format!("job {job}: capture held"));
    screen.press("q");
    assert_eq!(screen.exit(), Some(0), "{}", screen.text());

    let state = runtime.block_on(client.state()).unwrap();
    assert_eq!(state.server.phase, Phase::Serving);
    let running = state.job.unwrap();
    assert_eq!(running.id, job);
    assert!(running.end.is_none(), "{running:?}");
    assert!(!state.activity.is_empty(), "{:?}", state.activity);

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
