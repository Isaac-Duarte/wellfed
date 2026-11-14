//! Repository maintenance tasks, run as `cargo xtask <task>`.
//!
//! `fuzz` runs one cargo-fuzz target for a bounded time, or replays one
//! saved input.
//!
//! `check-repository` checks local documentation links, the docs' staleness
//! rules (`doc_checks`: commands, make targets, paths, status words, time-bound
//! wording, anchors), portable repository inputs, that no Python returns under `src/`, and the source rules (raw
//! output writers only through the HAL's owners, no board card ids), that
//! each diagram in `docs/` still matches its Mermaid, that every device API op
//! has a client in the CLI and the app, then the server's crate
//! layering from `cargo metadata`. It is a maintenance check, not a secret
//! scanner or a licensing audit.
//!
//! `package-desktop` builds the desktop app's installers for the host it
//! runs on, checks what each holds and writes their sums; `desktop-sums`
//! rewrites the sums over every host's artifacts gathered in one folder, and
//! `check-windows-installer` installs and uninstalls the Windows one.
//!
//! `bless-diagrams` records each diagram pair's hashes in
//! `docs/diagrams.lock`, once its author has checked the SVG and its Mermaid
//! still show the same thing.

#![forbid(unsafe_code)]

mod clap_tree;
mod desktop;
mod diagrams;
mod doc_checks;
mod fuzz;
mod layers;
mod op_clients;
mod repository;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(about = "Repository maintenance tasks")]
struct Cli {
    /// Repository checkout to check.
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[command(subcommand)]
    task: Task,
}

#[derive(Subcommand)]
enum Task {
    /// Documentation links, tool dependencies, portable inputs, source rules.
    CheckRepository,
    /// Record each docs diagram's SVG and Mermaid hashes, after checking they match.
    BlessDiagrams,
    /// One bounded libFuzzer run of a network-parser target (needs cargo-fuzz).
    Fuzz {
        #[arg(value_enum)]
        target: fuzz::Target,
        /// How long to fuzz: at least a second, since libFuzzer reads 0 as
        /// no limit.
        #[arg(
            long,
            default_value_t = 60,
            conflicts_with = "replay",
            value_parser = clap::value_parser!(u32).range(1..)
        )]
        seconds: u32,
        /// Run this one saved input (a crash artifact) instead of fuzzing.
        #[arg(long)]
        replay: Option<PathBuf>,
    },
    /// Build the desktop app's installers for this host into dist/desktop/<version>/.
    PackageDesktop {
        /// Package the release build already under src/app/build/ instead of building one.
        #[arg(long)]
        skip_build: bool,
        /// Linux: the appimagetool binary.
        #[arg(long)]
        appimagetool: Option<PathBuf>,
        /// Linux: the AppImage runtime appimagetool prepends.
        #[arg(long)]
        appimage_runtime: Option<PathBuf>,
        /// Windows: Inno Setup's ISCC.exe, if not where winget puts it.
        #[arg(long)]
        iscc: Option<PathBuf>,
    },
    /// Rewrite SHA256SUMS over every desktop artifact in dist/desktop/<version>/.
    DesktopSums,
    /// Install a Windows installer silently, check it, uninstall it, and check the stores survive.
    CheckWindowsInstaller {
        /// The Wellfed-<version>-windows-setup.exe to check.
        setup: PathBuf,
    },
}

/// Why `check-repository` could not run its checks at all.
#[derive(Debug, thiserror::Error)]
enum CheckError {
    #[error("could not list the checkout")]
    Files(#[from] std::io::Error),
    #[error("could not check the crate layering")]
    Layers(#[from] layers::Error),
}

/// The file checks, then the server's crate layering from `cargo metadata`.
fn check_repository(root: &Path) -> Result<repository::Summary, CheckError> {
    let mut summary = repository::check(root)?;
    summary.errors.extend(layers::check(root)?);
    Ok(summary)
}

/// An error and each of its causes, on one line.
fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(next) = cause {
        let _ = write!(text, ": {next}");
        cause = next.source();
    }
    text
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.task {
        Task::CheckRepository => match check_repository(&cli.root) {
            Ok(summary) if summary.errors.is_empty() => {
                println!(
                    "Repository check passed: {} files, {} local documentation links.",
                    summary.files, summary.links
                );
                ExitCode::SUCCESS
            }
            Ok(summary) => {
                eprintln!("{}", summary.errors.join("\n"));
                ExitCode::from(1)
            }
            Err(error) => {
                eprintln!("check-repository: {}", chain(&error));
                ExitCode::from(2)
            }
        },
        Task::BlessDiagrams => {
            let blessed = repository::listed(&cli.root)
                .map_err(|error| vec![error.to_string()])
                .and_then(|names| {
                    diagrams::bless(&cli.root, &names)
                        .map_err(|errors| errors.iter().map(|e| chain(e)).collect())
                });
            match blessed {
                Ok(count) => {
                    println!("Blessed {count} diagrams in {}.", diagrams::LOCK);
                    ExitCode::SUCCESS
                }
                Err(errors) => {
                    eprintln!("{}", errors.join("\n"));
                    ExitCode::from(1)
                }
            }
        }
        Task::PackageDesktop {
            skip_build,
            appimagetool,
            appimage_runtime,
            iscc,
        } => {
            let options = desktop::Options {
                build: if skip_build {
                    desktop::Build::Existing
                } else {
                    desktop::Build::Fresh
                },
                appimagetool,
                appimage_runtime,
                iscc,
            };
            match desktop::package(&cli.root, &options) {
                Ok(built) => {
                    for artifact in built {
                        println!("Built {}", artifact.display());
                    }
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("package-desktop: {}", chain(&error));
                    ExitCode::from(1)
                }
            }
        }
        Task::DesktopSums => match desktop::sums(&cli.root) {
            Ok(sums) => {
                println!("Wrote {}", sums.display());
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("desktop-sums: {}", chain(&error));
                ExitCode::from(1)
            }
        },
        Task::CheckWindowsInstaller { setup } => match desktop::check_windows_installer(&setup) {
            Ok(()) => {
                println!(
                    "{}: installed, checked, uninstalled; the stores are still there.",
                    setup.display()
                );
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("check-windows-installer: {}", chain(&error));
                ExitCode::from(1)
            }
        },
        Task::Fuzz {
            target,
            seconds,
            replay,
        } => {
            // The run starts in `--root`, so a relative input is resolved here.
            let replay = match replay.map(std::path::absolute).transpose() {
                Ok(replay) => replay,
                Err(error) => {
                    eprintln!("fuzz: {error}");
                    return ExitCode::from(2);
                }
            };
            let run = replay
                .as_deref()
                .map_or(fuzz::Run::Seconds(seconds), fuzz::Run::Replay);
            match fuzz::run(&cli.root, target, run) {
                Ok(status) if status.success() => ExitCode::SUCCESS,
                Ok(status) => {
                    eprintln!(
                        "fuzz: the run ended with {status}; any crash input is under src/fuzz/artifacts/"
                    );
                    ExitCode::from(1)
                }
                Err(error) => {
                    eprintln!("fuzz: {error}");
                    ExitCode::from(2)
                }
            }
        }
    }
}
