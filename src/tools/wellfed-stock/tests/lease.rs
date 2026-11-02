//! `setup-send` takes the bench lease for its one frame, beside its config as
//! it is named; a `serve` takes none, since it only answers the feeder.

mod common;

use std::path::Path;
use std::process::{Command, Output, Stdio};

use wellfed_formats::bench::lease::Holder;

/// The holder of the bench lease in `directory`, if any.
fn lease(directory: &Path) -> Option<Holder> {
    let data = std::fs::read(directory.join("bench.lease")).ok()?;
    Some(serde_json::from_slice(&data).unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_takes_no_lease() {
    let (dir, path, config) = common::configured();
    let _service = common::start(&path);
    let http = common::http();
    common::ready(&config, &http).await;

    assert!(lease(dir.path()).is_none());
}

/// Another process holding the bench, killed when the test ends, passed
/// or not.
struct Other(std::process::Child);

impl Drop for Other {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// `setup-send` on `config`; refused before it reads a passphrase or sends.
fn setup_send(config: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wellfed-stock"))
        .args(["setup-send", "--ssid", "s", "--id", "1"])
        .args([
            "--on-feeder-network",
            "PL-AF103-test",
            "--to",
            "127.0.0.1:9",
        ])
        .arg("--config")
        .arg(config)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// Hold the bench in `directory`, for as long as the result lives.
fn held(directory: &Path) -> Other {
    let other = Other(Command::new("sleep").arg("60").spawn().unwrap());
    let holder = Holder::of(other.0.id(), vec!["wellfed-bench".into(), "on".into()]).unwrap();
    wellfed_formats::private::write_json(&directory.join("bench.lease"), &holder).unwrap();
    other
}

#[test]
fn setup_send_refuses_a_held_bench() {
    let (dir, path, _) = common::configured();
    let _other = held(dir.path());

    let output = setup_send(&path);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("The bench is held by pid"), "{stderr}");
}

/// A worktree's `private/` linking the main checkout's config keeps its lease
/// in its own `private/`, beside the bench that runs from it.
#[test]
fn a_symlinked_config_looks_for_the_lease_where_it_is_named() {
    let (main, target, _) = common::configured();
    let worktree = tempfile::tempdir().unwrap();
    let link = worktree.path().join("server.json");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let _other = held(worktree.path());

    let output = setup_send(&link);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("The bench is held by pid"), "{stderr}");
    assert!(lease(main.path()).is_none());
}
