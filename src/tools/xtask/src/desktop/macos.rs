//! macOS: a disk image holding `Wellfed.app` and a link to Applications.
//!
//! The app is the release build as Flutter signs it, ad hoc, with no
//! developer ID and no notarization, so Gatekeeper asks before its first
//! open (the owner guide says how). A release build is universal, and the
//! check holds it to both architectures, the runner's and the bridge's.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use wellfed_formats::require;

use super::{APP, Build, Error, Version, at, flutter_build, output, require_built, status};

/// The release app, relative to the app's folder.
const RELEASE: &str = "build/macos/Build/Products/Release/Wellfed.app";
/// The runner, the bridge (a framework its pod links the static library
/// into), Dart's AOT code and the icon.
const CONTENTS: &[&str] = &[
    "Contents/MacOS/Wellfed",
    "Contents/Frameworks/wellfed_app_bridge.framework/wellfed_app_bridge",
    "Contents/Frameworks/App.framework/App",
    "Contents/Resources/AppIcon.icns",
    "Contents/Info.plist",
];
/// The binaries a universal build holds both slices of.
const UNIVERSAL: &[&str] = &[
    "Contents/MacOS/Wellfed",
    "Contents/Frameworks/wellfed_app_bridge.framework/wellfed_app_bridge",
];

pub(super) fn package(
    root: &Path,
    version: &Version,
    build: Build,
    stage: &Path,
    out: &Path,
) -> Result<PathBuf, Error> {
    if build == Build::Fresh {
        flutter_build(root, "macos")?;
    }
    let app = root.join(APP).join(RELEASE);
    require_built(&app, CONTENTS)?;
    check_app(&app)?;

    let folder = stage.join("dmg");
    fs::create_dir_all(&folder).map_err(at(&folder))?;
    // ditto keeps the bundle's links, modes and signature as they are.
    status(
        Command::new("ditto")
            .arg(&app)
            .arg(folder.join("Wellfed.app")),
    )?;
    super::symlink(Path::new("/Applications"), &folder.join("Applications"))?;

    let dmg = out.join(format!("Wellfed-{}-macos.dmg", version.name));
    status(
        Command::new("hdiutil")
            .args([
                "create",
                "-volname",
                "Wellfed",
                "-fs",
                "HFS+",
                "-format",
                "UDZO",
                "-ov",
                "-srcfolder",
            ])
            .arg(&folder)
            .arg(&dmg),
    )?;
    check_dmg(&dmg, stage)?;
    Ok(dmg)
}

/// The app's signature holds, and its runner and bridge are universal.
fn check_app(app: &Path) -> Result<(), Error> {
    output(
        Command::new("codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(app),
    )?;
    for binary in UNIVERSAL {
        let path = app.join(binary);
        let archs = output(Command::new("lipo").arg("-archs").arg(&path))?;
        let archs = String::from_utf8_lossy(&archs.stdout).into_owned();
        let words: Vec<&str> = archs.split_whitespace().collect();
        require(
            words.contains(&"arm64") && words.contains(&"x86_64"),
            || Error::Malformed {
                package: path.clone(),
                problem: format!("it holds {}, not arm64 and x86_64", archs.trim()),
            },
        )?;
    }
    Ok(())
}

/// Verifies the image's checksum, mounts it read-only, and checks the app
/// and the Applications link inside.
fn check_dmg(dmg: &Path, stage: &Path) -> Result<(), Error> {
    output(Command::new("hdiutil").arg("verify").arg(dmg))?;

    let mount = stage.join("mount");
    fs::create_dir_all(&mount).map_err(at(&mount))?;
    output(
        Command::new("hdiutil")
            .args([
                "attach",
                "-readonly",
                "-nobrowse",
                "-noautoopen",
                "-mountpoint",
            ])
            .arg(&mount)
            .arg(dmg),
    )?;
    let checked = check_mounted(dmg, &mount);
    let detached = output(Command::new("hdiutil").arg("detach").arg(&mount));
    checked?;
    detached.map(drop)
}

fn check_mounted(dmg: &Path, mount: &Path) -> Result<(), Error> {
    let app = mount.join("Wellfed.app");
    for entry in CONTENTS {
        require(app.join(entry).exists(), || Error::MissingEntry {
            package: dmg.to_path_buf(),
            entry: format!("Wellfed.app/{entry}"),
        })?;
    }
    let link = fs::read_link(mount.join("Applications")).ok();
    require(link.as_deref() == Some(Path::new("/Applications")), || {
        Error::MissingEntry {
            package: dmg.to_path_buf(),
            entry: "the Applications link".to_owned(),
        }
    })?;
    check_app(&app)
}
