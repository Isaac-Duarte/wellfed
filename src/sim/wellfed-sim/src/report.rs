//! The evidence one scenario run leaves: its events, assertions and the
//! identity of the worker and sources that produced them.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use wellfed_formats::canonical::sha256;

use crate::HarnessError;
use crate::protocol::{Event, Fault, Outcome, Profile, Reason, State, StoragePhase, Submission};

/// Hashes of the worker and of every source that shapes it.
#[derive(Debug, Serialize)]
pub(crate) struct Identity {
    worker_sha256: String,
    sources: BTreeMap<String, String>,
}

/// The crates whose sources decide what the worker does: the harness and
/// worker, the SDK's core and HAL the owner runs on, the feeding crates
/// (the owner, the journal, the controller), the fake SDK that is the
/// synthetic hardware, and the board the plant is measured from. A listed
/// crate that is gone refuses the report, so a move fails loudly rather
/// than leaving an empty walk.
const SOURCES: [&str; 10] = [
    "src/sim/wellfed-sim",
    "src/sdk/wellfed-core",
    "src/sdk/wellfed-core-feeder",
    "src/sdk/wellfed-hal",
    "src/sdk/wellfed-hal-common",
    "src/sdk/wellfed-hal-feeder",
    "src/sdk/wellfed-amebad-sys",
    "src/sdk/wellfed-testkit",
    "src/sdk/wellfed-board",
    "src/sdk/wellfed-board-plaf103",
];

pub(crate) fn identities(root: &Path, executable: &Path) -> Result<Identity, HarnessError> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in fs::read_dir(dir)?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name() != Some("target".as_ref()) {
                    walk(&path, out)?;
                }
            } else if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("rs" | "toml" | "h")
            ) {
                out.push(path);
            }
        }
        Ok(())
    }

    let mut paths = Vec::new();
    for source in SOURCES {
        walk(&root.join(source), &mut paths).map_err(|error| HarnessError::Sources {
            directory: source,
            error,
        })?;
    }
    paths.push(root.join("Cargo.lock"));

    let sources = paths
        .iter()
        .filter_map(|p| {
            let data = fs::read(p).ok()?;
            let name = p.strip_prefix(root).unwrap_or(p).display().to_string();
            Some((name, sha256(&data)))
        })
        .collect();

    Ok(Identity {
        worker_sha256: fs::read(executable).map(|d| sha256(&d)).unwrap_or_default(),
        sources,
    })
}

/// Notes the harness adds to the worker's events in the report.
#[derive(Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub(crate) enum Note {
    Boot { profile: Profile },
    Action { action: Value },
    PowerRemoved { outputs: &'static str },
    WatchdogResetRequested,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub(crate) enum Logged {
    Worker(Event),
    Harness(Note),
}

#[derive(Debug, Serialize)]
pub(crate) struct Entry {
    pub(crate) at_ms: u64,
    pub(crate) boot: u64,
    #[serde(flatten)]
    pub(crate) event: Logged,
}

#[derive(Debug, Serialize)]
pub(crate) struct Assertion {
    pub(crate) at_ms: u64,
    pub(crate) label: String,
    pub(crate) passed: bool,
    pub(crate) actual: Value,
    pub(crate) expected: Value,
}

/// The name tables for the indexes the report's events carry.
#[derive(Debug, Serialize)]
pub(crate) struct Enums {
    pub(crate) state: &'static [State],
    pub(crate) fault: &'static [Fault],
    pub(crate) outcome: &'static [Outcome],
    pub(crate) reason: &'static [Reason],
    pub(crate) submission: &'static [Submission],
    pub(crate) storage_phase: &'static [StoragePhase],
    pub(crate) motor: [&'static str; 4],
}

/// What a passing report does not establish.
#[derive(Debug, Serialize)]
pub(crate) struct Limits {
    pub(crate) synthetic_hardware: bool,
    pub(crate) plant_from_bench_measurements: bool,
    pub(crate) physical_reliability_verified: bool,
    pub(crate) full_boot_or_rtos_emulated: bool,
    pub(crate) real_flash_provider: bool,
    pub(crate) network_or_scheduler_implemented: bool,
}

/// One scenario's evidence. Serialize it through [`serde_json::to_value`]
/// so its keys come out sorted, as the reports always have.
#[derive(Debug, Serialize)]
pub struct Report {
    pub(crate) version: u32,
    pub name: String,
    pub(crate) profile: Profile,
    pub(crate) scenario_sha256: String,
    pub(crate) identity: Identity,
    pub(crate) enums: Enums,
    pub(crate) limits: Limits,
    pub(crate) events: Vec<Entry>,
    pub(crate) assertions: Vec<Assertion>,
    pub passed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The identity hashes the crates that decide a feed: the worker's own
    /// and the owner's among them.
    #[test]
    fn the_identity_hashes_the_worker_and_the_owner() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let root = root.canonicalize().unwrap();
        let identity = identities(&root, Path::new("/nonexistent")).unwrap();
        for source in [
            "src/sim/wellfed-sim/src/main.rs",
            "src/sdk/wellfed-hal-feeder/src/owner/mod.rs",
            "src/sdk/wellfed-core-feeder/src/feed_journal.rs",
            "Cargo.lock",
        ] {
            assert!(identity.sources.contains_key(source), "{source}");
        }
        for source in SOURCES {
            assert!(
                identity.sources.keys().any(|name| name.starts_with(source)),
                "{source}"
            );
        }
    }
}
