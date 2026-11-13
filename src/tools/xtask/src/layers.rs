//! The workspace's crate layering, checked from `cargo metadata`.
//!
//! The compiler keeps the layers apart only if each crate's dependencies
//! follow them, so a manifest change that adds a shortcut fails here instead
//! of in review:
//!
//! - the SDK's device-neutral crates link no kind's own crate by any path,
//!   so a fountain's image carries no feeder code
//!   (docs/firmware/device-kinds.md);
//! - a crate outside `src/tools/` links no host tool's crate, so what the
//!   app, Home Assistant and the firmware build on is a library in its own
//!   layer, never a tool's internals;
//! - the simulators in `src/sim/` are linked by the tools and by other
//!   crates' tests, never by a crate that ships, so no build an owner runs
//!   carries a simulated feeder.
//!
//! Dev-dependencies are exempt (a test may build a simulated feeder or run
//! a tool).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, io};

use serde::Deserialize;

/// The SDK crates every device kind builds on, which must not link a
/// kind's own crate ([`is_kind_sdk`]) by any path. The board crates are
/// among them: a board names its pins and safe outputs as data (the
/// PLAF103's motor and sensor pins too), which the kind's drivers read; it
/// needs none of a kind's policy.
const NEUTRAL_SDK: [&str; 11] = [
    "wellfed-amebad-sys",
    "wellfed-board",
    "wellfed-board-plaf103",
    "wellfed-board-plwf106",
    "wellfed-core",
    "wellfed-hal",
    "wellfed-hal-common",
    "wellfed-sdk",
    "wellfed-sdk-macros",
    "wellfed-session",
    "wellfed-signed",
];

/// A device kind's own SDK crates: its logic beside `wellfed-core` and its
/// drivers above the chip HAL.
fn is_kind_sdk(name: &str) -> bool {
    matches!(name, "wellfed-core-feeder" | "wellfed-hal-feeder")
}

/// The host tools' folder: a crate whose manifest is under it is a tool.
const TOOLS: &str = "src/tools/";

/// The simulators' folder: a crate whose manifest is under it is a
/// simulator.
const SIM: &str = "src/sim/";

/// Running or reading `cargo metadata` failed, so no rule could be checked.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("could not run cargo metadata")]
    Run(#[from] io::Error),
    #[error("cargo metadata failed ({status}): {stderr}")]
    Failed {
        status: std::process::ExitStatus,
        stderr: String,
    },
    #[error("could not read cargo metadata's output")]
    Parse(#[from] serde_json::Error),
    #[error("cargo metadata has no dependency graph")]
    NoResolve,
    #[error("cargo metadata names package {0} in its graph but not among its packages")]
    UnknownPackage(String),
}

/// The parts of `cargo metadata --format-version 1` the rules read.
#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    workspace_members: Vec<String>,
    workspace_root: PathBuf,
    resolve: Option<Resolve>,
}

#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
    manifest_path: PathBuf,
}

#[derive(Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}

#[derive(Deserialize)]
struct Node {
    id: String,
    deps: Vec<NodeDep>,
}

#[derive(Deserialize)]
struct NodeDep {
    pkg: String,
    dep_kinds: Vec<DepKind>,
}

#[derive(Deserialize)]
struct DepKind {
    /// `null` for a normal dependency.
    kind: Option<Kind>,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Dev,
    Build,
}

/// How a dependency reaches its dependent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Use {
    /// A normal or build dependency: it is linked into the crate.
    Links,
    /// A dev-dependency only: it is linked into the crate's tests.
    TestsOnly,
}

/// The resolved graph, by package name. A name can belong to several
/// versions of an external crate, so edges run between package ids.
struct Graph {
    names: BTreeMap<String, String>,
    members: BTreeSet<String>,
    /// The members whose manifest is under [`TOOLS`].
    tools: BTreeSet<String>,
    /// The members whose manifest is under [`SIM`].
    sims: BTreeSet<String>,
    edges: BTreeMap<String, Vec<(String, Use)>>,
}

impl Graph {
    fn from_metadata(metadata: Metadata) -> Result<Self, Error> {
        let resolve = metadata.resolve.ok_or(Error::NoResolve)?;
        let under = |folder: &str| -> BTreeSet<String> {
            metadata
                .packages
                .iter()
                .filter(|package| {
                    package
                        .manifest_path
                        .strip_prefix(&metadata.workspace_root)
                        .is_ok_and(|path| path.starts_with(folder))
                })
                .map(|package| package.id.clone())
                .collect()
        };
        let tools = under(TOOLS);
        let sims = under(SIM);
        let names: BTreeMap<String, String> = metadata
            .packages
            .into_iter()
            .map(|package| (package.id, package.name))
            .collect();

        let mut edges = BTreeMap::new();
        for node in resolve.nodes {
            let mut deps = Vec::with_capacity(node.deps.len());
            for dep in node.deps {
                if !names.contains_key(&dep.pkg) {
                    return Err(Error::UnknownPackage(dep.pkg));
                }
                let links = dep
                    .dep_kinds
                    .iter()
                    .any(|kind| kind.kind != Some(Kind::Dev));
                let used = if links { Use::Links } else { Use::TestsOnly };
                deps.push((dep.pkg, used));
            }
            edges.insert(node.id, deps);
        }

        let members: BTreeSet<String> = metadata.workspace_members.into_iter().collect();
        Ok(Self {
            names,
            tools: members.intersection(&tools).cloned().collect(),
            sims: members.intersection(&sims).cloned().collect(),
            members,
            edges,
        })
    }

    fn name<'a>(&'a self, id: &'a str) -> &'a str {
        self.names.get(id).map_or(id, String::as_str)
    }

    fn deps(&self, id: &str) -> &[(String, Use)] {
        self.edges.get(id).map_or(&[], Vec::as_slice)
    }

    /// The shortest linked path from `from` to a package whose name
    /// `target` accepts, as package names, or `None` when none is linked.
    fn linked_path(&self, from: &str, target: impl Fn(&str) -> bool) -> Option<Vec<String>> {
        let mut parent: BTreeMap<&str, &str> = BTreeMap::new();
        let mut queue = VecDeque::from([from]);

        while let Some(id) = queue.pop_front() {
            if id != from && target(self.name(id)) {
                let mut path = vec![self.name(id).to_string()];
                let mut at = id;
                while let Some(&up) = parent.get(at) {
                    path.push(self.name(up).to_string());
                    at = up;
                }
                path.reverse();
                return Some(path);
            }
            for (dep, used) in self.deps(id) {
                if *used == Use::Links && dep != from && !parent.contains_key(dep.as_str()) {
                    parent.insert(dep, id);
                    queue.push_back(dep);
                }
            }
        }

        None
    }
}

/// Run `cargo metadata` over the workspace at `root` and check every rule.
pub(crate) fn check(root: &Path) -> Result<Vec<String>, Error> {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--all-features",
            "--locked",
        ])
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err(Error::Failed {
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    let metadata: Metadata = serde_json::from_slice(&output.stdout)?;
    Ok(rules(&Graph::from_metadata(metadata)?))
}

/// Every broken rule, each naming the edge (or path) that breaks it.
fn rules(graph: &Graph) -> Vec<String> {
    let mut errors = Vec::new();

    for member in &graph.members {
        let from = graph.name(member);

        for (dep, used) in graph.deps(member) {
            if *used == Use::TestsOnly || !graph.members.contains(dep) {
                continue;
            }
            let to = graph.name(dep);

            if !graph.tools.contains(member) && graph.tools.contains(dep) {
                errors.push(format!(
                    "dependency rule: {from} → {to}: a crate outside {TOOLS} links no host tool's crate"
                ));
            }
            if !graph.tools.contains(member)
                && !graph.sims.contains(member)
                && graph.sims.contains(dep)
            {
                errors.push(format!(
                    "dependency rule: {from} → {to}: outside {TOOLS} and {SIM}, only tests link a simulator"
                ));
            }
        }

        if NEUTRAL_SDK.contains(&from)
            && let Some(path) = graph.linked_path(member, is_kind_sdk)
        {
            errors.push(format!(
                "dependency rule: {}: the device-neutral SDK links no kind's crate",
                path.join(" → ")
            ));
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A graph whose package ids are their names, from `(from, to, use)`
    /// edges; `members` are the workspace's crates.
    fn graph(members: &[&str], edges: &[(&str, &str, Use)]) -> Graph {
        let mut graph = Graph {
            names: BTreeMap::new(),
            members: members.iter().map(ToString::to_string).collect(),
            tools: members
                .iter()
                .filter(|name| TOOL_MEMBERS.contains(name))
                .map(ToString::to_string)
                .collect(),
            sims: members
                .iter()
                .filter(|name| SIM_MEMBERS.contains(name))
                .map(ToString::to_string)
                .collect(),
            edges: BTreeMap::new(),
        };
        for (from, to, used) in edges {
            for name in [from, to] {
                graph.names.insert(name.to_string(), name.to_string());
            }
            graph
                .edges
                .entry(from.to_string())
                .or_default()
                .push((to.to_string(), *used));
        }
        graph
    }

    /// The tests' members under `src/tools/`.
    const TOOL_MEMBERS: [&str; 4] = [
        "wellfed",
        "wellfed-bench",
        "wellfed-build",
        "wellfed-stock-server",
    ];

    /// The tests' members under `src/sim/`.
    const SIM_MEMBERS: [&str; 3] = ["wellfed-device-sim", "wellfed-feeder-sim", "wellfed-sim"];

    #[test]
    fn the_neutral_sdk_links_no_kind_s_crate() {
        use Use::{Links, TestsOnly};
        let members = [
            "wellfed-core",
            "wellfed-core-feeder",
            "wellfed-hal",
            "wellfed-hal-feeder",
            "wellfed-sdk",
            "wellfed-fw-feeder",
        ];
        let mut edges = vec![
            ("wellfed-core-feeder", "wellfed-core", Links),
            ("wellfed-hal", "wellfed-core", Links),
            ("wellfed-hal-feeder", "wellfed-hal", Links),
            ("wellfed-hal-feeder", "wellfed-core-feeder", Links),
            ("wellfed-sdk", "wellfed-hal", Links),
            ("wellfed-fw-feeder", "wellfed-sdk", Links),
            ("wellfed-fw-feeder", "wellfed-hal-feeder", Links),
            // A neutral crate's tests may build the feeder's.
            ("wellfed-core", "wellfed-core-feeder", TestsOnly),
        ];
        assert!(rules(&graph(&members, &edges)).is_empty());

        edges.push(("wellfed-hal", "wellfed-hal-feeder", Links));
        assert_eq!(
            rules(&graph(&members, &edges)),
            [
                "dependency rule: wellfed-hal → wellfed-hal-feeder: the device-neutral SDK links no kind's crate",
                "dependency rule: wellfed-sdk → wellfed-hal → wellfed-hal-feeder: the device-neutral SDK links no kind's crate",
            ]
        );
    }

    #[test]
    fn nothing_outside_the_tools_links_a_tool() {
        use Use::{Links, TestsOnly};
        let members = [
            "wellfed-install",
            "wellfed-stock",
            "wellfed-stock-server",
            "wellfed-build",
            "wellfed-feeder-sim",
        ];
        let mut edges = vec![
            ("wellfed-install", "wellfed-stock", Links),
            ("wellfed-stock-server", "wellfed-stock", Links),
            ("wellfed-build", "wellfed-stock", Links),
            // A test of a crate outside the tools may run one.
            ("wellfed-install", "wellfed-build", TestsOnly),
        ];
        assert!(rules(&graph(&members, &edges)).is_empty());

        edges.extend([
            ("wellfed-install", "wellfed-build", Links),
            // A simulator is outside the tools too.
            ("wellfed-feeder-sim", "wellfed-stock-server", Links),
        ]);
        assert_eq!(
            rules(&graph(&members, &edges)),
            [
                "dependency rule: wellfed-feeder-sim → wellfed-stock-server: a crate outside src/tools/ links no host tool's crate",
                "dependency rule: wellfed-install → wellfed-build: a crate outside src/tools/ links no host tool's crate",
            ]
        );
    }

    #[test]
    fn only_tools_and_tests_link_a_simulator() {
        use Use::{Links, TestsOnly};
        let members = [
            "wellfed",
            "wellfed-app-testkit",
            "wellfed-core",
            "wellfed-device-sim",
            "wellfed-feeder-sim",
            "wellfed-fw-feeder",
            "wellfed-install",
            "wellfed-sim",
        ];
        let mut edges = vec![
            // A simulator builds on another, and on the code it simulates.
            ("wellfed-feeder-sim", "wellfed-device-sim", Links),
            ("wellfed-feeder-sim", "wellfed-fw-feeder", Links),
            ("wellfed-feeder-sim", "wellfed-core", Links),
            // The tools run it, and other crates' tests may.
            ("wellfed", "wellfed-feeder-sim", Links),
            ("wellfed-install", "wellfed-feeder-sim", TestsOnly),
            ("wellfed-fw-feeder", "wellfed-sim", TestsOnly),
        ];
        assert!(rules(&graph(&members, &edges)).is_empty());

        edges.extend([
            ("wellfed-core", "wellfed-sim", Links),
            ("wellfed-fw-feeder", "wellfed-sim", Links),
            // A test kit is no exception: a shipped crate could link it and
            // carry the simulator into its build.
            ("wellfed-app-testkit", "wellfed-feeder-sim", Links),
        ]);
        assert_eq!(
            rules(&graph(&members, &edges)),
            [
                "dependency rule: wellfed-app-testkit → wellfed-feeder-sim: outside src/tools/ and src/sim/, only tests link a simulator",
                "dependency rule: wellfed-core → wellfed-sim: outside src/tools/ and src/sim/, only tests link a simulator",
                "dependency rule: wellfed-fw-feeder → wellfed-sim: outside src/tools/ and src/sim/, only tests link a simulator",
            ]
        );
    }

    #[test]
    fn metadata_kinds_decide_what_links() {
        let json = r#"{
            "packages": [
                {"id": "a 0.1.0", "name": "wellfed-install", "manifest_path": "/w/src/client/wellfed-install/Cargo.toml"},
                {"id": "b 3.0.0", "name": "tempfile", "manifest_path": "/r/tempfile/Cargo.toml"},
                {"id": "c 0.8.0", "name": "axum", "manifest_path": "/r/axum/Cargo.toml"},
                {"id": "d 1.0.0", "name": "cc", "manifest_path": "/r/cc/Cargo.toml"},
                {"id": "e 0.1.0", "name": "wellfed-bench", "manifest_path": "/w/src/tools/wellfed-bench/Cargo.toml"},
                {"id": "f 0.1.0", "name": "wellfed-feeder-sim", "manifest_path": "/w/src/sim/wellfed-feeder-sim/Cargo.toml"}
            ],
            "workspace_members": ["a 0.1.0", "e 0.1.0", "f 0.1.0"],
            "workspace_root": "/w",
            "resolve": {"nodes": [
                {"id": "a 0.1.0", "deps": [
                    {"pkg": "b 3.0.0", "dep_kinds": [{"kind": "dev", "target": null}]},
                    {"pkg": "c 0.8.0", "dep_kinds": [{"kind": "dev", "target": null}]},
                    {"pkg": "d 1.0.0", "dep_kinds": [{"kind": "build", "target": null}]},
                    {"pkg": "e 0.1.0", "dep_kinds": [{"kind": "dev", "target": null}]},
                    {"pkg": "f 0.1.0", "dep_kinds": [{"kind": null, "target": null}, {"kind": "dev", "target": null}]}
                ]}
            ]}
        }"#;
        let metadata: Metadata = serde_json::from_str(json).unwrap();
        let graph = Graph::from_metadata(metadata).unwrap();

        assert_eq!(
            graph.deps("a 0.1.0"),
            [
                ("b 3.0.0".to_string(), Use::TestsOnly),
                ("c 0.8.0".to_string(), Use::TestsOnly),
                ("d 1.0.0".to_string(), Use::Links),
                ("e 0.1.0".to_string(), Use::TestsOnly),
                ("f 0.1.0".to_string(), Use::Links),
            ]
        );
        assert_eq!(graph.tools, BTreeSet::from(["e 0.1.0".to_string()]));
        assert_eq!(graph.sims, BTreeSet::from(["f 0.1.0".to_string()]));
        assert_eq!(
            rules(&graph),
            [
                "dependency rule: wellfed-install → wellfed-feeder-sim: outside src/tools/ and src/sim/, only tests link a simulator"
            ]
        );
    }
}
