//! `cargo xtask package-desktop`: the desktop app's installers, built from a
//! release build on the host they are for, since Flutter cannot cross-build a
//! desktop: an AppImage and a `.deb` on Linux, a disk image on macOS, and an
//! Inno Setup installer on Windows.
//!
//! Each run builds the app (unless told to package the build already there),
//! stages it with the files that package needs from `src/app/packaging/`,
//! packs it, then opens the result again and checks that it holds the app,
//! the Rust bridge and the icon. The artifacts land in
//! `dist/desktop/<version>/`, named from the pubspec's version, with a
//! `SHA256SUMS` over every artifact in that folder, so the other hosts'
//! artifacts copied in and `cargo xtask desktop-sums` give one list.
//!
//! None of it signs anything: the beta's installers are unsigned, and the
//! owner guide says how each system opens an unsigned app.

mod linux;
mod macos;
mod windows;

use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use wellfed_formats::canonical::sha256;
use wellfed_formats::require;

pub(crate) use windows::check_installer as check_windows_installer;

/// The app, relative to the repository root.
const APP: &str = "src/app";
/// The packaging inputs: desktop entry, AppStream metadata, Debian control
/// and the Inno Setup script.
const PACKAGING: &str = "src/app/packaging";
/// The app's id everywhere a system names it: the Linux runner's GApplication
/// id and data folder, the Mac's bundle id and the Windows toast id.
const APP_ID: &str = "io.github.isaacduarte.wellfed";
/// Where the artifacts go (`dist/desktop/<version>/`) and each host stages
/// its packages' trees (`dist/stage/`, rebuilt on every run), ignored by Git.
const DIST: &str = "dist";
/// The sums file over every artifact in a version's folder.
const SUMS: &str = "SHA256SUMS";

/// What `package-desktop` was asked to package.
#[derive(Debug)]
pub(crate) struct Options {
    /// Package the release build already under `src/app/build/` rather than
    /// building one.
    pub build: Build,
    /// Linux: the appimagetool binary.
    pub appimagetool: Option<PathBuf>,
    /// Linux: the AppImage runtime appimagetool prepends, so it fetches
    /// nothing at build time.
    pub appimage_runtime: Option<PathBuf>,
    /// Windows: Inno Setup's compiler, `ISCC.exe`.
    pub iscc: Option<PathBuf>,
}

/// Whether to run `flutter build` first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Build {
    Fresh,
    Existing,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("{path}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not start {program}")]
    Spawn {
        program: String,
        #[source]
        source: io::Error,
    },
    #[error("{program} failed ({status}): {stderr}")]
    Failed {
        program: String,
        status: std::process::ExitStatus,
        stderr: String,
    },
    #[error("{0} has no `version: <x.y.z>[+build]` line")]
    NoVersion(PathBuf),
    #[error("the pubspec's version {0:?} is not x.y.z or x.y.z-pre")]
    BadVersion(String),
    #[error("desktop packages are built on Linux, macOS or Windows, not {0}")]
    UnsupportedHost(&'static str),
    #[error("the release build lacks {0}: build it, or drop --skip-build")]
    MissingBuild(PathBuf),
    #[error("{tool} is needed: {hint}")]
    MissingTool {
        tool: &'static str,
        hint: &'static str,
    },
    #[error("{package} lacks {entry}")]
    MissingEntry { package: PathBuf, entry: String },
    #[error("{0}: its runtime is not a 64-bit little-endian ELF")]
    RuntimeNotElf(PathBuf),
    #[error("{package}: its members are {members:?}, not dpkg's three in order")]
    DebMembers {
        package: PathBuf,
        members: Vec<String>,
    },
    #[error("{binary} holds {archs}, not both arm64 and x86_64")]
    NotUniversal { binary: PathBuf, archs: String },
    #[error("{binary} needs glibc {needs}, newer than the packages' floor of {floor}")]
    GlibcTooNew {
        binary: PathBuf,
        needs: String,
        floor: String,
    },
    #[error("{path} has SHA-256 {found}, not the pinned {expected}: download it again from {url}")]
    PinMismatch {
        path: PathBuf,
        found: String,
        expected: &'static str,
        url: &'static str,
    },
    #[error("git did not give the last commit's time: {0:?}")]
    NoCommitTime(String),
    #[error(
        "Wellfed is already installed for this user (at {0}); uninstall it before the check, which would remove it"
    )]
    AlreadyInstalled(PathBuf),
    #[error("the uninstaller left {0}, or its uninstall entry or shortcut, after two minutes")]
    NotRemoved(PathBuf),
    #[error("the uninstall removed something from the app's stores in {0}")]
    DataRemoved(PathBuf),
    #[error("{0} is not set")]
    NoEnvironment(&'static str),
}

/// A path's IO error, with the path.
fn at(path: &Path) -> impl FnOnce(io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// The app's version from `pubspec.yaml`, without its build number.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Version {
    /// `0.1.0` or `0.1.0-beta.1`, as the artifacts are named.
    name: String,
    /// The pubspec's build number, `1` when it has none.
    build: u32,
}

impl Version {
    fn read(root: &Path) -> Result<Self, Error> {
        let path = root.join(APP).join("pubspec.yaml");
        let text = fs::read_to_string(&path).map_err(at(&path))?;
        let line = text
            .lines()
            .find_map(|line| line.strip_prefix("version:"))
            .ok_or_else(|| Error::NoVersion(path.clone()))?;
        Self::parse(line.trim())
    }

    fn parse(text: &str) -> Result<Self, Error> {
        let (name, build) = text.split_once('+').unwrap_or((text, "1"));
        let (core, pre) = name
            .split_once('-')
            .map_or((name, None), |(c, p)| (c, Some(p)));
        let numbers: Vec<&str> = core.split('.').collect();
        let numeric = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
        let pre_ok = pre.is_none_or(|pre| {
            !pre.is_empty() && pre.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.')
        });
        require(
            numbers.len() == 3 && numbers.iter().all(|n| numeric(n)) && pre_ok,
            || Error::BadVersion(text.to_owned()),
        )?;
        let build = build
            .parse()
            .map_err(|_| Error::BadVersion(text.to_owned()))?;

        Ok(Self {
            name: name.to_owned(),
            build,
        })
    }

    /// Debian sorts `~` before the release, as `-` sorts in semver.
    fn debian(&self) -> String {
        self.name.replacen('-', "~", 1)
    }

    /// Windows's four-number file version: the three numbers and the build.
    fn windows_file(&self) -> String {
        let core = self.name.split('-').next().unwrap_or(&self.name);
        format!("{core}.{}", self.build)
    }
}

/// Builds this host's packages, checks them and rewrites the sums.
pub(crate) fn package(root: &Path, options: &Options) -> Result<Vec<PathBuf>, Error> {
    // Tools run in the staging folders, so every path they get is absolute.
    let root = &std::path::absolute(root).map_err(at(root))?;
    let version = Version::read(root)?;
    let out = root.join(DIST).join("desktop").join(&version.name);
    fs::create_dir_all(&out).map_err(at(&out))?;
    let stage = root.join(DIST).join("stage");
    if stage.exists() {
        fs::remove_dir_all(&stage).map_err(at(&stage))?;
    }
    fs::create_dir_all(&stage).map_err(at(&stage))?;

    let built = match std::env::consts::OS {
        "linux" => linux::package(root, &version, options, &stage, &out)?,
        "macos" => vec![macos::package(root, &version, options.build, &stage, &out)?],
        "windows" => vec![windows::package(root, &version, options, &stage, &out)?],
        other => return Err(Error::UnsupportedHost(other)),
    };

    write_sums(&out)?;
    Ok(built)
}

/// Rewrites the sums in this version's folder, over whatever artifacts
/// were gathered there.
pub(crate) fn sums(root: &Path) -> Result<PathBuf, Error> {
    let version = Version::read(root)?;
    write_sums(&root.join(DIST).join("desktop").join(version.name))
}

/// Rewrites `SHA256SUMS` in a version's folder over every artifact in it,
/// in `sha256sum` format, sorted by name.
fn write_sums(folder: &Path) -> Result<PathBuf, Error> {
    let mut names = Vec::new();
    for entry in fs::read_dir(folder).map_err(at(folder))? {
        let entry = entry.map_err(at(folder))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_artifact(&name) {
            names.push(name);
        }
    }
    names.sort();

    let mut text = String::new();
    for name in names {
        let path = folder.join(&name);
        let bytes = fs::read(&path).map_err(at(&path))?;
        let _ = writeln!(text, "{}  {name}", sha256(&bytes));
    }
    let sums = folder.join(SUMS);
    fs::write(&sums, text).map_err(at(&sums))?;
    Ok(sums)
}

/// The names the four packages are given.
fn is_artifact(name: &str) -> bool {
    (name.starts_with("Wellfed-")
        && (name.ends_with("-macos.dmg")
            || name.ends_with("-x86_64.AppImage")
            || name.ends_with("-windows-setup.exe")))
        || (name.starts_with("wellfed_") && name.ends_with("_amd64.deb"))
}

/// Runs `flutter build <platform> --release` in the app.
fn flutter_build(root: &Path, platform: &str) -> Result<(), Error> {
    let mut command = Command::new(program("flutter"));
    command
        .args(["build", platform, "--release"])
        .current_dir(root.join(APP));
    // CocoaPods needs a UTF-8 locale.
    if platform == "macos" {
        command
            .env("LANG", "en_US.UTF-8")
            .env("LC_ALL", "en_US.UTF-8");
    }
    status(&mut command)
}

/// Windows runs `flutter` and other scripts through their `.bat`.
fn program(name: &str) -> String {
    if cfg!(windows) && name == "flutter" {
        format!("{name}.bat")
    } else {
        name.to_owned()
    }
}

/// Runs a command with its output passed through; fails on a non-zero exit.
fn status(command: &mut Command) -> Result<(), Error> {
    let program = command.get_program().to_string_lossy().into_owned();
    let status = command.status().map_err(|source| Error::Spawn {
        program: program.clone(),
        source,
    })?;
    require(status.success(), || Error::Failed {
        program,
        status,
        stderr: String::new(),
    })
}

/// Runs a command and returns its output; fails on a non-zero exit.
fn output(command: &mut Command) -> Result<Output, Error> {
    let program = command.get_program().to_string_lossy().into_owned();
    let output = command.output().map_err(|source| Error::Spawn {
        program: program.clone(),
        source,
    })?;
    require(output.status.success(), || Error::Failed {
        program: program.clone(),
        status: output.status,
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    })?;
    Ok(output)
}

/// Fails unless every one of `paths` exists under `dir`: the release build's
/// own check before anything is staged.
fn require_built(dir: &Path, paths: &[&str]) -> Result<(), Error> {
    for path in paths {
        let full = dir.join(path);
        require(full.exists(), || Error::MissingBuild(full.clone()))?;
    }
    Ok(())
}

/// Fails unless each of `wanted` is in a package's listing.
fn require_listed(package: &Path, listing: &[String], wanted: &[&str]) -> Result<(), Error> {
    for entry in wanted {
        require(listing.iter().any(|line| line == entry), || {
            Error::MissingEntry {
                package: package.to_path_buf(),
                entry: (*entry).to_owned(),
            }
        })?;
    }
    Ok(())
}

/// Copies a tree, keeping symbolic links as links and file modes as they are.
fn copy_tree(from: &Path, to: &Path) -> Result<(), Error> {
    fs::create_dir_all(to).map_err(at(to))?;
    for entry in fs::read_dir(from).map_err(at(from))? {
        let entry = entry.map_err(at(from))?;
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        let kind = entry.file_type().map_err(at(&source))?;
        if kind.is_dir() {
            copy_tree(&source, &target)?;
        } else if kind.is_symlink() {
            let link = fs::read_link(&source).map_err(at(&source))?;
            symlink(&link, &target)?;
        } else {
            fs::copy(&source, &target).map_err(at(&source))?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn symlink(link: &Path, at_path: &Path) -> Result<(), Error> {
    std::os::unix::fs::symlink(link, at_path).map_err(at(at_path))
}

/// Windows's build holds no links, and nothing here makes one there.
#[cfg(not(unix))]
fn symlink(_link: &Path, at_path: &Path) -> Result<(), Error> {
    Err(Error::Io {
        path: at_path.to_path_buf(),
        source: io::Error::from(io::ErrorKind::Unsupported),
    })
}

/// Reads a packaging template and fills in its `@NAME@` fields.
fn template(root: &Path, relative: &str, fields: &[(&str, &str)]) -> Result<String, Error> {
    let path = root.join(PACKAGING).join(relative);
    let mut text = fs::read_to_string(&path).map_err(at(&path))?;
    for (name, value) in fields {
        text = text.replace(&format!("@{name}@"), value);
    }
    Ok(text)
}

/// The last commit's time, as `SOURCE_DATE_EPOCH` unless that is set: the
/// time every packed file is given, so a rebuild of the same commit packs the
/// same bytes.
fn source_date_epoch(root: &Path) -> Result<u64, Error> {
    if let Some(epoch) = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|text| text.parse().ok())
    {
        return Ok(epoch);
    }
    let out = output(
        Command::new("git")
            .args(["log", "-1", "--format=%ct"])
            .current_dir(root),
    )?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    text.parse().map_err(|_| Error::NoCommitTime(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_drop_the_build_number() {
        let version = Version::parse("0.1.0+1").expect("a pubspec version");
        assert_eq!(version.name, "0.1.0");
        assert_eq!(version.debian(), "0.1.0");
        assert_eq!(version.windows_file(), "0.1.0.1");

        let beta = Version::parse("1.0.0-beta.2+7").expect("a prerelease");
        assert_eq!(beta.name, "1.0.0-beta.2");
        assert_eq!(beta.debian(), "1.0.0~beta.2");
        assert_eq!(beta.windows_file(), "1.0.0.7");

        assert_eq!(Version::parse("2.3.4").expect("no build").build, 1);
    }

    #[test]
    fn versions_refuse_what_a_file_name_cannot_carry() {
        for bad in [
            "0.1",
            "0.1.0.0",
            "0.1.x",
            "0.1.0-",
            "0.1.0-beta/1",
            "0.1.0+b",
        ] {
            assert!(
                matches!(Version::parse(bad), Err(Error::BadVersion(_))),
                "{bad} should be refused"
            );
        }
    }

    #[test]
    fn sums_cover_only_the_artifacts() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        for name in [
            "Wellfed-0.1.0-x86_64.AppImage",
            "wellfed_0.1.0_amd64.deb",
            "Wellfed-0.1.0-macos.dmg",
            "Wellfed-0.1.0-windows-setup.exe",
            "notes.txt",
        ] {
            fs::write(folder.path().join(name), name).expect("a file");
        }

        let sums = write_sums(folder.path()).expect("sums");
        let text = fs::read_to_string(sums).expect("the sums");
        let names: Vec<&str> = text.lines().filter_map(|l| l.split("  ").nth(1)).collect();
        assert_eq!(
            names,
            [
                "Wellfed-0.1.0-macos.dmg",
                "Wellfed-0.1.0-windows-setup.exe",
                "Wellfed-0.1.0-x86_64.AppImage",
                "wellfed_0.1.0_amd64.deb",
            ]
        );
        assert!(text.starts_with(&sha256(b"Wellfed-0.1.0-macos.dmg")));
    }
}
