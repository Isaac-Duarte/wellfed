//! The cargo environment of a firmware build, and the rustc it selects.
//!
//! Overrides that can change firmware code generation or the compiler are
//! refused rather than scrubbed: refusing keeps an explicit user choice
//! visible, while silently dropping it would make the build differ from what
//! the caller asked for. `cargo run` sets `RUSTUP_TOOLCHAIN`, so the builder
//! is run as its own executable, never through cargo.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

use regex::Regex;
use wellfed_formats::manifest::RustCompiler;
use wellfed_formats::require;

use crate::platform::TARGETS;
use crate::{Repo, Result, text, which};

/// An override that would change the firmware compilation, or a compiler
/// other than the pinned one.
#[derive(Debug, thiserror::Error)]
pub enum EnvironmentError {
    /// Environment variables, sorted, that a firmware build refuses.
    #[error("Unset compiler/profile overrides before a firmware build: {}", names.join(", "))]
    Overrides { names: Vec<String> },
    /// The dotted keys of a cargo configuration file that a build refuses.
    #[error(
        "Cargo configuration overrides firmware compilation in {}: {}",
        path.display(),
        keys.join(", ")
    )]
    CargoConfig { path: PathBuf, keys: Vec<String> },
    /// A cargo configuration file that is not TOML, which cargo would refuse
    /// too, but whose overrides nothing could then rule out.
    #[error("Cargo configuration {} is not TOML: {source}", path.display())]
    CargoConfigUnreadable {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("rustc selected from the repository does not match rust-toolchain.toml")]
    Channel,
    #[error("rustc printed nothing")]
    NoVersion,
}

const RUST_ENVIRONMENT: [&str; 13] = [
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_BUILD_RUSTFLAGS",
    "RUSTC",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "CARGO_BUILD_RUSTC",
    "CARGO_BUILD_RUSTC_WRAPPER",
    "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
    "RUSTUP_TOOLCHAIN",
    "RUSTC_BOOTSTRAP",
    "CARGO_INCREMENTAL",
    "CARGO_BUILD_INCREMENTAL",
];
const RUST_ENVIRONMENT_PREFIXES: [&str; 2] = ["CARGO_PROFILE_", "CARGO_UNSTABLE_"];
/// The board `wellfed-board` builds for, which a firmware build sets from
/// its device's model, never from the caller.
pub(crate) const BOARD_VARIABLE: &str = "WELLFED_BOARD";
/// The update key `wellfed-hal` compiles in, which a build sets from its
/// provisioning or leaves unset.
pub(crate) const OTA_KEY_VARIABLE: &str = "WELLFED_OTA_PUBLIC_KEY";
/// Set, with no meaningful value, exactly when the image has build Wi-Fi
/// (a bench image with credentials); the feeder runtime reads its presence
/// (`network::BUILD_WIFI`), and the generated header's presence macro has
/// the same name.
pub(crate) const BUILD_WIFI_VARIABLE: &str = "WELLFED_BUILD_WIFI";
/// Variables every firmware build sets itself: a cargo `[env]` default for
/// one is replaced, but a forced entry would replace the build's value.
const ALWAYS_SET: [&str; 4] = [
    BOARD_VARIABLE,
    "WELLFED_BUILD_ID",
    "WELLFED_SDK_DIR",
    "WELLFED_NEWLIB_INCLUDE",
];
/// Variables a firmware build sets only sometimes: any cargo `[env]` entry
/// for one, forced or not, would supply a value where the build sets none.
const SOMETIMES_SET: [&str; 2] = [OTA_KEY_VARIABLE, BUILD_WIFI_VARIABLE];

pub(crate) type Environment = BTreeMap<String, String>;

/// The Rust compiler a Rust profile builds with, resolved before and
/// compared after compilation.
pub(crate) struct Rust {
    pub(crate) environment: Environment,
    pub(crate) compiler: RustCompiler,
}

impl Rust {
    /// This process's environment, checked, and the rustc it selects.
    pub(crate) fn resolve(repo: &Repo) -> Result<Self> {
        let environment = rust_environment(&repo.root, process_environment())?;
        let compiler = rust_compiler(repo, &environment)?;
        Ok(Self {
            environment,
            compiler,
        })
    }
}

/// `CARGO_TARGET_<TRIPLE>_RUSTFLAGS` of a firmware target.
fn target_rustflags(name: &str) -> bool {
    TARGETS.iter().any(|target| {
        let triple = target.to_ascii_uppercase().replace(['-', '.'], "_");
        name == format!("CARGO_TARGET_{triple}_RUSTFLAGS")
    })
}
/// Environment names that would change the firmware compilation, sorted.
fn forbidden_variables(environment: &Environment) -> Vec<String> {
    environment
        .keys()
        .filter(|name| {
            RUST_ENVIRONMENT.contains(&name.as_str())
                || RUST_ENVIRONMENT_PREFIXES
                    .iter()
                    .any(|prefix| name.starts_with(prefix))
                || target_rustflags(name)
        })
        .cloned()
        .collect()
}

/// Cargo configuration files that apply to a build run from the root.
fn cargo_config_files(root: &Path, environment: &Environment) -> Vec<PathBuf> {
    let home = environment
        .get("CARGO_HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            environment
                .get("HOME")
                .map(|home| Path::new(home).join(".cargo"))
        })
        .unwrap_or_else(|| PathBuf::from(".cargo"));

    let mut candidates: Vec<PathBuf> = root
        .ancestors()
        .flat_map(|dir| ["config.toml", "config"].map(|name| dir.join(".cargo").join(name)))
        .collect();
    candidates.extend(["config.toml", "config"].map(|name| home.join(name)));

    let mut seen = std::collections::HashSet::new();
    candidates
        .into_iter()
        .filter(|path| seen.insert(path.clone()) && path.is_file())
        .collect()
}

/// Dotted keys that can change firmware codegen, the selected compiler or a
/// variable the build sets, read with the TOML parser cargo uses.
///
/// Host-only `[target.<host triple>]` settings (such as a host linker) are
/// allowed, as is an unforced `[env]` default for a variable every build
/// sets (the repository's own board default).
fn cargo_config_overrides(text: &str) -> std::result::Result<Vec<String>, toml::de::Error> {
    let config: toml::Table = text.parse()?;

    let mut found = Vec::new();
    for (head, value) in &config {
        match head.as_str() {
            "profile" | "unstable" => leaves(head, value, &mut found),
            "build" => match value.as_table() {
                Some(build) => found.extend(
                    build
                        .keys()
                        .filter(|key| BUILD_KEYS.contains(&key.as_str()))
                        .map(|key| format!("build.{key}")),
                ),
                None => found.push(head.clone()),
            },
            "target" => {
                let Some(targets) = value.as_table() else {
                    found.push(head.clone());
                    continue;
                };
                for (selector, settings) in targets {
                    let firmware =
                        selector.starts_with("cfg(") || TARGETS.contains(&selector.as_str());
                    let rustflags = settings
                        .as_table()
                        .is_none_or(|settings| settings.contains_key("rustflags"));
                    if firmware && rustflags {
                        found.push(format!("target.{selector}.rustflags"));
                    }
                }
            }
            "env" => {
                let Some(variables) = value.as_table() else {
                    found.push(head.clone());
                    continue;
                };
                found.extend(
                    variables
                        .iter()
                        .filter(|(name, value)| forbidden_env(name, value))
                        .map(|(name, _)| format!("env.{name}")),
                );
            }
            _ => {}
        }
    }
    Ok(found)
}

/// The `[build]` keys that choose the compiler or its flags.
const BUILD_KEYS: [&str; 5] = [
    "rustflags",
    "rustc",
    "rustc-wrapper",
    "rustc-workspace-wrapper",
    "incremental",
];

/// Each leaf below `prefix`, dotted.
fn leaves(prefix: &str, value: &toml::Value, found: &mut Vec<String>) {
    match value.as_table() {
        Some(table) if !table.is_empty() => {
            for (key, value) in table {
                leaves(&format!("{prefix}.{key}"), value, found);
            }
        }
        _ => found.push(prefix.to_string()),
    }
}

/// Whether a cargo `[env]` entry could change a variable the build sets.
/// Cargo replaces a variable already set only when the entry is forced (a
/// table with `force = true`); a plain string or an unforced table is a
/// default.
fn forbidden_env(name: &str, value: &toml::Value) -> bool {
    let forced = value
        .get("force")
        .and_then(toml::Value::as_bool)
        .unwrap_or(false);
    SOMETIMES_SET.contains(&name) || ALWAYS_SET.contains(&name) && forced
}

/// The cargo environment, refusing output-affecting overrides in it or in
/// any cargo configuration file that applies.
pub(crate) fn rust_environment(root: &Path, environment: Environment) -> Result<Environment> {
    let names = forbidden_variables(&environment);
    require(names.is_empty(), || EnvironmentError::Overrides { names })?;

    for path in cargo_config_files(root, &environment) {
        let keys = match cargo_config_overrides(&String::from_utf8_lossy(&fs::read(&path)?)) {
            Ok(keys) => keys,
            Err(source) => {
                return Err(EnvironmentError::CargoConfigUnreadable { path, source }.into());
            }
        };
        require(keys.is_empty(), || EnvironmentError::CargoConfig {
            path,
            keys,
        })?;
    }
    Ok(environment)
}

/// This process's environment.
pub(crate) fn process_environment() -> Environment {
    std::env::vars().collect()
}

/// The rustc cargo resolves from the root, which must be the pinned channel.
/// Recorded in the manifest and compared again after the build.
pub(crate) fn rust_compiler(repo: &Repo, environment: &Environment) -> Result<RustCompiler> {
    static CHANNEL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?m)^channel\s*=\s*"([^"]+)""#).expect("literal pattern"));
    static RELEASE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?m)^release:\s*(\S+)$").expect("literal pattern"));

    let pinned = fs::read_to_string(repo.path("rust-toolchain.toml"))?;
    let verbose = text(
        Command::new("rustc")
            .arg("-vV")
            .current_dir(&repo.root)
            .env_clear()
            .envs(environment),
    )?;
    let channel = CHANNEL.captures(&pinned).map(|c| c[1].to_string());
    let release = RELEASE.captures(&verbose).map(|c| c[1].to_string());
    require(channel.is_some() && channel == release, || {
        EnvironmentError::Channel
    })?;

    let path = match which("rustup", environment.get("PATH").map(String::as_str)) {
        Some(_) => Some(
            text(
                Command::new("rustup")
                    .args(["which", "rustc"])
                    .current_dir(&repo.root)
                    .env_clear()
                    .envs(environment),
            )?
            .trim()
            .to_string(),
        ),
        None => None,
    };
    let version = verbose.lines().next().ok_or(EnvironmentError::NoVersion)?;
    Ok(RustCompiler {
        version: version.to_string(),
        verbose: verbose.trim().lines().map(str::to_string).collect(),
        path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    fn clean() -> Environment {
        [("PATH", "/usr/bin"), ("HOME", "/nonexistent")]
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .into()
    }

    #[test]
    fn output_affecting_overrides_are_refused() {
        let root = tempfile::tempdir().unwrap();
        assert!(rust_environment(root.path(), clean()).is_ok());

        for name in [
            "RUSTFLAGS",
            "CARGO_ENCODED_RUSTFLAGS",
            "RUSTC_WRAPPER",
            "RUSTUP_TOOLCHAIN",
            "CARGO_PROFILE_FIRMWARE_OPT_LEVEL",
            "CARGO_TARGET_THUMBV8M_MAIN_NONE_EABIHF_RUSTFLAGS",
        ] {
            let mut environment = clean();
            environment.insert(name.into(), "x".into());
            let error = rust_environment(root.path(), environment).unwrap_err();
            assert!(
                matches!(
                    &error,
                    Error::Environment(EnvironmentError::Overrides { names }) if names == &[name]
                ),
                "{name}: {error}"
            );
        }

        let mut host = clean();
        host.insert("CARGO_BUILD_TARGET_DIR".into(), "/tmp/x".into());
        host.insert(
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS".into(),
            "-Clink-arg=x".into(),
        );
        assert!(rust_environment(root.path(), host).is_ok());
    }

    #[test]
    fn cargo_configuration_overrides() {
        let overrides = |text: &str| cargo_config_overrides(text).unwrap();
        let allowed = "# [build]\n# rustc-wrapper = \"sccache\"\n[target.x86_64-unknown-linux-gnu]\n\
                       linker = \"clang\"\nrustflags = [\"-Clink-arg=--ld-path=mold\"]\n[build]\njobs = 4\n\
                       [target.'thumbv8m.main-none-eabihf']\nrunner = \"probe\"\n";
        assert!(overrides(allowed).is_empty());
        // An unforced default for a variable every build sets, as the
        // repository's own configuration sets the board, never replaces the
        // one the build sets.
        assert!(
            overrides(
                "[env]\nWELLFED_BOARD = \"plaf103\"\nWELLFED_BUILD_ID = { value = \"x\" }\n\
                 OTHER = { value = \"x\", force = true }\n"
            )
            .is_empty()
        );
        let repository = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../.cargo/config.toml");
        assert!(overrides(&fs::read_to_string(repository).unwrap()).is_empty());

        for (text, key) in [
            (
                "[build]\nrustflags = [\"-Copt-level=0\"]\n",
                "build.rustflags",
            ),
            ("build.rustc-wrapper = \"sccache\"\n", "build.rustc-wrapper"),
            ("build = { rustflags = [\"x\"] }\n", "build.rustflags"),
            (
                "[profile.firmware]\nopt-level = 0\n",
                "profile.firmware.opt-level",
            ),
            (
                "[target.'thumbv8m.base-none-eabi']\nrustflags = [\n  \"x\",\n]\n",
                "target.thumbv8m.base-none-eabi.rustflags",
            ),
            (
                "[target.'cfg(target_os = \"none\")']\nrustflags = [\"x\"]\n",
                "target.cfg(target_os = \"none\").rustflags",
            ),
            (
                "target = { 'thumbv8m.main-none-eabihf' = { rustflags = [\"x\"] } }\n",
                "target.thumbv8m.main-none-eabihf.rustflags",
            ),
            ("[unstable]\nbuild-std = [\"core\"]\n", "unstable.build-std"),
            (
                "[env]\nWELLFED_BOARD = { value = \"plwf106\", force = true }\n",
                "env.WELLFED_BOARD",
            ),
            (
                "[env.WELLFED_BOARD]\nvalue = \"plwf106\"\nforce = true\n",
                "env.WELLFED_BOARD",
            ),
            (
                "env = { WELLFED_BOARD = { value = \"plwf106\", force = true } }\n",
                "env.WELLFED_BOARD",
            ),
            (
                "[env]\n\"WELLFED_BUILD_ID\" = { value = \"x\", force = true }\n",
                "env.WELLFED_BUILD_ID",
            ),
            (
                "[env]\nWELLFED_SDK_DIR = { value = \"/x\", force = true, relative = false }\n",
                "env.WELLFED_SDK_DIR",
            ),
            // The update key and the build Wi-Fi flag are refused even as a
            // default: a build that sets neither would take the default.
            (
                "[env]\nWELLFED_OTA_PUBLIC_KEY = { value = \"k2\", force = true }\n",
                "env.WELLFED_OTA_PUBLIC_KEY",
            ),
            (
                "[env.WELLFED_OTA_PUBLIC_KEY]\nvalue = \"k2\"\nforce = true\n",
                "env.WELLFED_OTA_PUBLIC_KEY",
            ),
            (
                "[env]\nWELLFED_OTA_PUBLIC_KEY = \"k2\"\n",
                "env.WELLFED_OTA_PUBLIC_KEY",
            ),
            (
                "[env]\nWELLFED_BUILD_WIFI = \"1\"\n",
                "env.WELLFED_BUILD_WIFI",
            ),
            ("env = \"x\"\n", "env"),
        ] {
            assert_eq!(overrides(text), [key], "{text}");
        }

        assert!(cargo_config_overrides("[env\nx = 1\n").is_err());
    }

    #[test]
    fn configuration_files_are_refused_with_their_path() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join(".cargo");
        fs::create_dir(&config).unwrap();
        fs::write(
            config.join("config.toml"),
            "[profile.firmware]\nlto = false\n",
        )
        .unwrap();

        let error = rust_environment(root.path(), clean()).unwrap_err();
        let Error::Environment(EnvironmentError::CargoConfig { path, keys }) = &error else {
            panic!("{error}");
        };
        assert_eq!(keys, &["profile.firmware.lto"]);
        assert_eq!(path, &config.join("config.toml"));
    }
}
