//! `wellfed install` takes the bench lease beside its stock config before it
//! touches anything, so it refuses a bench another tool holds.

use std::process::Command;

use wellfed_formats::bench::lease::Holder;

/// Another process holding the bench, killed when the test ends, passed or
/// not.
struct Other(std::process::Child);

impl Drop for Other {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn install_refuses_a_held_bench() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("installer.json");
    std::fs::write(
        &config,
        r#"{"api_token":"t","http_listen":"127.0.0.1:18765","mqtt_listen":"127.0.0.1:18883"}"#,
    )
    .unwrap();

    let other = Other(Command::new("sleep").arg("60").spawn().unwrap());
    let holder = Holder::of(other.0.id(), vec!["wellfed-bench".into(), "on".into()]).unwrap();
    wellfed_formats::private::write_json(&dir.path().join("bench.lease"), &holder).unwrap();

    // No bench server for it to find, whoever runs the test.
    let output = Command::new(env!("CARGO_BIN_EXE_wellfed"))
        .args(["install", "--review", "absent", "--manifest", "absent"])
        .arg("--stock-config")
        .arg(&config)
        .env("XDG_RUNTIME_DIR", dir.path())
        .env("HOME", dir.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("The bench is held by pid"), "{stderr}");
}
