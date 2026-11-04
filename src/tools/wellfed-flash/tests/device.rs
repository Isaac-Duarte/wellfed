//! The `wellfed-flash` writers against a model this client holds no pins
//! for: `update`, `give-way` and `stock-recovery` with `--device plwf106`
//! are refused before the session starts, so no port is opened. The port
//! and every evidence path named do not exist, so nothing here can reach a
//! device, and the refusal names the model rather than a missing file.

use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::Value;

const BINARY: &str = env!("CARGO_BIN_EXE_wellfed-flash");
const NO_PORT: &str = "/nonexistent/wellfed-flash-test-port";

/// The writer's summary and exit code for `args` with the PLWF106 named.
fn refused(root: &Path, args: &[&str]) -> (Option<i32>, Value) {
    let evidence = [
        "--device",
        "plwf106",
        "--port",
        NO_PORT,
        "--baseline",
        "private/none-baseline.bin",
        "--rom-report",
        "private/none-rom.json",
        "--small-report",
        "private/none-small.json",
        "--output",
        "private/none-output",
    ];
    let output = Command::new(BINARY)
        .arg("--root")
        .arg(root)
        .args(args)
        .args(evidence)
        .current_dir(root)
        .env_remove("WELLFED_FLASH_EMULATOR")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let summary = serde_json::from_str(stdout.lines().last().unwrap()).unwrap();
    (output.status.code(), summary)
}

#[test]
fn every_writer_refuses_a_model_without_pins_before_serial_access() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("private")).unwrap();

    for args in [
        &["update", "inspect", "--target", "private/none-review"][..],
        &["give-way", "inspect", "--fallback", "private/none-review"][..],
        &[
            "stock-recovery",
            "prepare",
            "--current",
            "private/none-read.bin",
        ][..],
    ] {
        let (code, summary) = refused(root.path(), args);
        assert_eq!(code, Some(2), "{args:?}: {summary}");
        assert_eq!(summary["status"], "refused", "{args:?}: {summary}");
        let error = summary["error"].as_str().unwrap();
        assert!(error.contains("plwf106"), "{args:?}: {error}");
    }
}
