//! What the terminal tests share: a process on a pseudo-terminal that
//! `script` provides, with its screen collected as it comes.

#![allow(
    dead_code,
    reason = "each test binary compiles this whole and uses only some of it"
)]

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process};

pub const BENCH: &str = env!("CARGO_BIN_EXE_wellfed-bench");

/// Shell `line` on a terminal of its own.
pub struct Screen {
    pub child: Child,
    seen: Arc<Mutex<Vec<u8>>>,
}

impl Screen {
    /// `line` runs after the terminal is given a size, as `script` leaves
    /// one with none when its own input is no terminal.
    pub fn start(line: &str, home: &std::path::Path) -> Self {
        let line = format!("stty cols 130 rows 46; {line}");
        let mut child = Command::new("script")
            .args(["-qfec", &line, "/dev/null"])
            .env("SHELL", "/bin/sh")
            // A bench server finds its clients through these: never the
            // real one's, on a host that has one.
            .env("HOME", home)
            .env("XDG_RUNTIME_DIR", home)
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

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.seen.lock().unwrap()).into_owned()
    }

    /// Wait for `text` on the screen. The terminal is sent only the cells
    /// that change, with cursor moves over the blanks, so the two are
    /// compared with no escape sequences or spaces.
    pub fn until_shown(&self, text: &str) {
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

    pub fn press(&mut self, key: &str) {
        let stdin = self.child.stdin.as_mut().unwrap();
        stdin.write_all(key.as_bytes()).unwrap();
        stdin.flush().unwrap();
    }

    /// Close the terminal under whatever runs on it, as a dropped `ssh -t`
    /// does: the kernel hangs it up and sends SIGHUP.
    pub fn hang_up(&mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }

    /// Its exit code, which it must reach soon.
    pub fn exit(&mut self) -> Option<i32> {
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

/// Whether `pid` is still running. A process that has ended and waits for
/// its new parent to reap it is not.
pub fn running(pid: Pid) -> bool {
    std::fs::read_to_string(format!("/proc/{}/stat", pid.as_raw_nonzero()))
        .ok()
        .and_then(|stat| {
            let (_, after_name) = stat.rsplit_once(')')?;
            after_name
                .split_whitespace()
                .next()
                .map(|state| state != "Z")
        })
        .unwrap_or(false)
}

/// Fail unless `pid` ends within `limit`, taking it down first so that a
/// failure here leaves nothing spinning on the host.
pub fn assert_ends(pid: Pid, limit: Duration, what: &str) {
    let deadline = Instant::now() + limit;
    while running(pid) {
        if Instant::now() >= deadline {
            let _ = kill_process(pid, Signal::KILL);
            panic!("{what}: pid {pid:?} still running after {limit:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
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
