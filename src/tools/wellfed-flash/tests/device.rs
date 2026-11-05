//! The `wellfed-flash` writers against the limits of each model's pins: a
//! fountain session asks for bank 0, a fallback review or an original read,
//! or a feeder give-way names no fallback. Each is refused before the
//! session reads any evidence, so no port is opened. The port and every
//! evidence path named do not exist, so nothing here can reach a device,
//! and the refusal names the model rather than a missing file.

use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::Value;

const BINARY: &str = env!("CARGO_BIN_EXE_wellfed-flash");
const NO_PORT: &str = "/nonexistent/wellfed-flash-test-port";

/// The writer's summary and exit code for `args` on `model`.
fn refused(root: &Path, model: &str, args: &[&str]) -> (Option<i32>, Value) {
    let evidence = [
        "--device",
        model,
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

/// A read that continues a ROM inspection names the model whose console it
/// showed; without one it is refused before the port opens.
#[test]
fn a_read_continuing_a_rom_inspection_needs_the_model() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("private")).unwrap();

    for mode in ["small", "full", "status"] {
        let output = Command::new(BINARY)
            .arg("--root")
            .arg(root.path())
            .args([
                "read",
                mode,
                "--port",
                NO_PORT,
                "--output",
                "private/none-output",
            ])
            .args(["--rom-report", "private/none-rom.json"])
            .current_dir(root.path())
            .env_remove("WELLFED_FLASH_EMULATOR")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let summary: Value = serde_json::from_str(stdout.lines().last().unwrap()).unwrap();
        assert_eq!(output.status.code(), Some(2), "{mode}: {summary}");
        let error = summary["error"].as_str().unwrap();
        assert!(error.contains("--device"), "{mode}: {error}");
        assert!(!root.path().join("private/none-output").exists());
    }
}

#[test]
fn each_model_is_refused_what_its_pins_do_not_cover_before_serial_access() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("private")).unwrap();

    let review = "private/none-review";
    for (model, args, named) in [
        // The fountain's bank 0 is the stock image its loader falls back to.
        (
            "plwf106",
            &["update", "inspect", "--bank", "0", "--target", review][..],
            "bank 0",
        ),
        // Its bank 1 original comes from its backup, and its fallback is
        // its stock bank 0, so neither takes a file of its own.
        (
            "plwf106",
            &[
                "update",
                "inspect",
                "--target",
                review,
                "--original-read",
                "private/none-read.bin",
            ][..],
            "--original-read",
        ),
        (
            "plwf106",
            &["give-way", "inspect", "--fallback", review][..],
            "fallback",
        ),
        // The feeder's give-way needs the review its bank 0 must hold.
        ("plaf103", &["give-way", "inspect"][..], "--fallback"),
    ] {
        let (code, summary) = refused(root.path(), model, args);
        assert_eq!(code, Some(2), "{args:?}: {summary}");
        assert_eq!(summary["status"], "refused", "{args:?}: {summary}");
        let error = summary["error"].as_str().unwrap();
        assert!(
            error.contains(model) && error.contains(named),
            "{args:?}: {error}"
        );
        assert!(!root.path().join("private/none-output").exists());
    }
}
