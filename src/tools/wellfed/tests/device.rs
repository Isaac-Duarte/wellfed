//! The device API path as a process. Against a feeder of a newer build
//! than this one, which answers every feeder write in a value this build
//! cannot read, a write may have run, so the command exits 3 rather than 0
//! or 1, says so, and never sends it again. Against the simulated feeder,
//! `wellfed feed` exits by how its feed ended: 1 for one that failed, and 3
//! for one whose result never reached it, which is sent once all the same.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use wellfed_feeder_sim::console;
use wellfed_feeder_sim::harness::Threaded;
use wellfed_feeder_sim::harness::newer::Newer;
use wellfed_feeder_sim::serve::Control;
use wellfed_model::{EventBody, FeedOutcome, FeederEvent, Pushed};
use wellfed_session::hex::Lower;
use wellfed_session::noise::Keypair;

/// The command's exit status for a write whose outcome is unknown.
const UNKNOWN: i32 = 3;
const OWNER: [u8; 32] = [0x22; 32];

/// This computer's key file, naming the feeder `identity`, in a directory
/// no bench server's pointer is looked for in.
struct Home {
    dir: tempfile::TempDir,
    keys: PathBuf,
}

impl Home {
    fn new(identity: &[u8; 32]) -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let keys = dir.path().join("wellfed-owner.json");
        let file = serde_json::json!({
            "version": 1,
            "secret": Lower(&OWNER).to_string(),
            "label": "laptop",
            "feeders": [{ "identity": Lower(identity).to_string() }],
        });
        std::fs::write(&keys, file.to_string()).expect("the key file is written");
        // As every writer makes a key file: the CLI refuses one others can read.
        std::fs::set_permissions(
            &keys,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
        )
        .expect("the key file is made private");
        Self { dir, keys }
    }

    /// `wellfed <args> --address <address>` with this key file. No bench
    /// server can lend its session here: its pointers are looked for under
    /// these directories only.
    fn command(&self, args: &[&str], address: std::net::SocketAddr) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_wellfed"));
        command
            .args(args)
            .args(["--address", &address.to_string()])
            .arg("--keys")
            .arg(&self.keys)
            .env("HOME", self.path())
            .env("WELLFED_HOME", self.path())
            .env_remove("XDG_RUNTIME_DIR");
        command
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }
}

fn owner() -> [u8; 32] {
    *Keypair::from_secret(OWNER).public()
}

#[test]
fn a_write_answered_unreadably_exits_unknown_and_is_not_sent_again() {
    let feeder = Newer::start();
    let home = Home::new(&feeder.identity);

    let ran = home
        .command(&["lock", "on"], feeder.address)
        .output()
        .expect("the CLI runs");

    let stderr = String::from_utf8_lossy(&ran.stderr);
    assert_eq!(ran.status.code(), Some(UNKNOWN), "{stderr}");
    assert!(stderr.contains("cannot read"), "{stderr}");
    assert!(stderr.contains("may have run"), "{stderr}");
    assert_eq!(feeder.writes().len(), 1, "the lock went once");
}

/// A feed that ended without its portions is the command's failure, not
/// a success that printed its result.
#[test]
fn a_feed_that_fails_exits_1() {
    let feeder = Threaded::start(owner());
    let home = Home::new(&feeder.identity);
    for control in console::parse("jam").expect("a console line") {
        feeder.control(control);
    }

    let ran = home
        .command(&["feed", "--portions", "3"], feeder.address)
        .output()
        .expect("the CLI runs");

    let stderr = String::from_utf8_lossy(&ran.stderr);
    assert_eq!(ran.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("the feed ended failed"), "{stderr}");
}

/// A feed whose `Fed` never reaches the command (the feeder restarts while
/// it runs) may have dropped anything up to its portions: the command
/// exits 3, says so, and sends no second feed.
#[test]
fn a_feed_whose_result_is_lost_exits_unknown_and_is_sent_once() {
    let feeder = Threaded::start(owner());
    let home = Home::new(&feeder.identity);
    feeder.control(Control::StallFeeds(true));

    let mut child = home
        .command(&["feed", "--portions", "2"], feeder.address)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the CLI starts");
    let stdout = child.stdout.take().expect("a piped standard output");
    let (lines, heard) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = lines.send(line);
        }
    });
    // The feed's answer is printed once it started: restart the feeder
    // then, with the feed held running.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let line = heard.recv_timeout(left).expect("the feed starts");
        if line.contains("feeding") {
            break;
        }
    }
    feeder.control(Control::Reboot);

    let ran = finished(child, Duration::from_secs(90));
    let stderr = String::from_utf8_lossy(&ran.stderr);
    assert_eq!(ran.status.code(), Some(UNKNOWN), "{stderr}");
    assert!(stderr.contains("may still be running"), "{stderr}");

    // The restart resolved the one feed it interrupted; none came after.
    let feeds = fed(&feeder);
    assert_eq!(feeds, [(1, FeedOutcome::Uncertain)], "one feed, sent once");
}

/// `child`'s end within `wait`; a child still running then is killed and
/// the test fails.
fn finished(mut child: std::process::Child, wait: Duration) -> Output {
    let deadline = Instant::now() + wait;
    while child.try_wait().expect("the child's status").is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("the CLI did not end within {} s", wait.as_secs());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    child.wait_with_output().expect("the CLI's output")
}

/// Every feed the feeder's history records, by number and end.
fn fed(feeder: &Threaded) -> Vec<(u32, FeedOutcome)> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime builds");
    runtime.block_on(async {
        let keypair = Keypair::from_secret(OWNER);
        let mut session = None;
        // The restarted feeder takes a moment to listen again.
        for _ in 0..50 {
            match wellfed_connect::Connection::open(
                &keypair,
                feeder.identity,
                feeder.address,
                [9; 32],
            )
            .await
            {
                Ok(opened) => {
                    session = Some(opened);
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
        let mut session = session.expect("a session after the restart");
        session.subscribe(None, false).await.expect("the history");

        let mut feeds = Vec::new();
        while let Ok(pushed) = session.next(Duration::from_millis(500)).await {
            if let Pushed::Event(event) = pushed
                && let EventBody::Feeder(FeederEvent::Fed { feed, outcome, .. }) = event.body
            {
                feeds.push((feed, outcome));
            }
        }
        feeds
    })
}
