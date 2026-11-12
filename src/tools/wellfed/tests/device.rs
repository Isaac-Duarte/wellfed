//! The device API path as a process, against a feeder of a newer build
//! than this one, which answers every feeder write in a value this build
//! cannot read: the write may have run, so the command exits 3 rather than
//! 0 or 1, says so, and never sends it again.

use std::process::Command;

use wellfed_feeder_sim::harness::newer::Newer;

/// The command's exit status for a write whose outcome is unknown.
const UNKNOWN: i32 = 3;

#[test]
fn a_write_answered_unreadably_exits_unknown_and_is_not_sent_again() {
    let feeder = Newer::start();
    let home = tempfile::tempdir().expect("a temporary directory");
    let keys = home.path().join("wellfed-owner.json");
    let file = serde_json::json!({
        "version": 1,
        "secret": "22".repeat(32),
        "label": "laptop",
        "feeders": [{ "identity": wellfed_session::hex::Lower(&feeder.identity).to_string() }],
    });
    std::fs::write(&keys, file.to_string()).expect("the key file is written");

    // No bench server can lend its session here: its pointers are looked
    // for under these directories only.
    let ran = Command::new(env!("CARGO_BIN_EXE_wellfed"))
        .args(["lock", "on", "--address", &feeder.address.to_string()])
        .arg("--keys")
        .arg(&keys)
        .env("HOME", home.path())
        .env("WELLFED_HOME", home.path())
        .env_remove("XDG_RUNTIME_DIR")
        .output()
        .expect("the CLI runs");

    let stderr = String::from_utf8_lossy(&ran.stderr);
    assert_eq!(ran.status.code(), Some(UNKNOWN), "{stderr}");
    assert!(stderr.contains("cannot read"), "{stderr}");
    assert!(stderr.contains("may have run"), "{stderr}");
    assert_eq!(feeder.writes().len(), 1, "the lock went once");
}
