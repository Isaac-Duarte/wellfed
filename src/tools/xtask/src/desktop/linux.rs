//! Linux: an AppImage and a `.deb` from one release bundle.
//!
//! Both carry the bundle whole under `usr/lib/wellfed/`, since the runner
//! finds its `lib/` and `data/` beside its own executable. Neither carries
//! GTK, libsecret or glibc: they come from the system, as the `.deb`'s
//! `Depends` names, and an AppImage runs only where they are installed.
//!
//! The `.deb` is packed with `tar`, `xz` and `ar` rather than `dpkg-deb`, so
//! it builds on any distribution without root. Every packed file gets the
//! last commit's time, owner root and normalized modes, and `ar` runs in its
//! deterministic mode, so the same bundle packs to the same bytes. The
//! AppImage's squashfs takes its times from `SOURCE_DATE_EPOCH` too.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use wellfed_formats::canonical::sha256;
use wellfed_formats::require;

use super::{
    APP, APP_ID, Error, Options, Version, at, copy_tree, flutter_build, output, require_built,
    require_listed, source_date_epoch, status, template,
};

/// The release bundle, relative to the app.
const BUNDLE: &str = "build/linux/x64/release/bundle";
/// Where both packages put the bundle.
const INSTALL: &str = "usr/lib/wellfed";
/// The brand icon, the largest PNG the brand folder has.
const ICON: &str = "assets/brand/png/app-icon-512.png";
/// What the release bundle must hold before anything is packed.
const BUILT: &[&str] = &[
    "wellfed",
    "lib/libwellfed_app_bridge.so",
    "lib/libapp.so",
    "lib/libflutter_linux_gtk.so",
    "data/icudtl.dat",
    "data/flutter_assets",
];

/// appimagetool 1.9.1 and the runtime of type2-runtime release 20251108,
/// both MIT. Pinned here, so a replaced download fails before it runs or is
/// embedded in a package; `make appimage-tools` downloads from these URLs
/// and checks these hashes, read from this file.
const APPIMAGETOOL: Pin = Pin {
    url: "https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage",
    sha256: "ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0",
};
const APPIMAGE_RUNTIME: Pin = Pin {
    url: "https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64",
    sha256: "2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d",
};

/// The newest glibc symbol version any packed binary may need, and the
/// `.deb`'s `libc6 (>= …)`. The build host's glibc sets what the Rust bridge
/// links against (2.39 on the Arch builder in November 2026), so a build on
/// a newer host fails here instead of shipping a package that won't load on
/// the distributions the owner guide names.
const GLIBC_FLOOR: (u32, u32) = (2, 39);

/// Builds, packs and checks the AppImage and the `.deb`.
pub(super) fn package(
    root: &Path,
    version: &Version,
    options: &Options,
    stage: &Path,
    out: &Path,
) -> Result<Vec<PathBuf>, Error> {
    let appimagetool = options.appimagetool.clone().ok_or(Error::MissingTool {
        tool: "appimagetool",
        hint: "pass --appimagetool (src/app/packaging/README.md)",
    })?;
    let runtime = options.appimage_runtime.clone().ok_or(Error::MissingTool {
        tool: "the AppImage runtime",
        hint: "pass --appimage-runtime (src/app/packaging/README.md)",
    })?;
    if options.build == super::Build::Fresh {
        flutter_build(root, "linux")?;
    }
    let bundle = root.join(APP).join(BUNDLE);
    require_built(&bundle, BUILT)?;
    let epoch = source_date_epoch(root)?;

    let appimage = out.join(format!("Wellfed-{}-x86_64.AppImage", version.name));
    let appdir = stage.join("Wellfed.AppDir");
    stage_appdir(root, version, &bundle, &appdir, epoch)?;
    check_glibc(&appdir.join(INSTALL))?;
    require_pinned(&appimagetool, APPIMAGETOOL)?;
    require_pinned(&runtime, APPIMAGE_RUNTIME)?;
    pack_appimage(&appimagetool, &runtime, &appdir, &appimage, epoch)?;
    check_appimage(&appimage)?;

    let deb = out.join(format!("wellfed_{}_amd64.deb", version.debian()));
    let deb_stage = stage.join("deb");
    stage_deb(root, version, &bundle, &deb_stage, epoch)?;
    check_glibc(&deb_stage.join("root").join(INSTALL))?;
    pack_deb(&deb_stage, &deb, epoch)?;
    check_deb(&deb)?;

    Ok(vec![appimage, deb])
}

/// The files both packages share under `usr/`: the bundle, the desktop entry
/// (whose `Exec` differs), the icon and the AppStream metadata.
fn stage_usr(
    root: &Path,
    version: &Version,
    bundle: &Path,
    tree: &Path,
    exec: &str,
    epoch: u64,
) -> Result<(), Error> {
    copy_tree(bundle, &tree.join(INSTALL))?;

    let desktop = template(
        root,
        "linux/io.github.isaacduarte.wellfed.desktop",
        &[("EXEC", exec)],
    )?;
    write(
        &tree.join(format!("usr/share/applications/{APP_ID}.desktop")),
        desktop.as_bytes(),
    )?;

    let icon = root.join(ICON);
    let icon_bytes = fs::read(&icon).map_err(at(&icon))?;
    write(
        &tree.join(format!("usr/share/icons/hicolor/512x512/apps/{APP_ID}.png")),
        &icon_bytes,
    )?;

    // AppStream reads both names; appimagetool looks only for `.appdata.xml`.
    let date = date_of(epoch);
    let metainfo = template(
        root,
        "linux/io.github.isaacduarte.wellfed.appdata.xml",
        &[("VERSION", &version.name), ("DATE", &date)],
    )?;
    write(
        &tree.join(format!("usr/share/metainfo/{APP_ID}.appdata.xml")),
        metainfo.as_bytes(),
    )?;

    lint(tree)
}

/// The AppDir: the shared tree, plus the entry, icon and `AppRun` at its top
/// that the AppImage runtime and desktop integrations look for.
fn stage_appdir(
    root: &Path,
    version: &Version,
    bundle: &Path,
    appdir: &Path,
    epoch: u64,
) -> Result<(), Error> {
    stage_usr(root, version, bundle, appdir, "wellfed", epoch)?;

    let desktop = format!("{APP_ID}.desktop");
    let icon = format!("{APP_ID}.png");
    super::symlink(
        Path::new(&format!("usr/share/applications/{desktop}")),
        &appdir.join(&desktop),
    )?;
    super::symlink(
        Path::new(&format!("usr/share/icons/hicolor/512x512/apps/{icon}")),
        &appdir.join(&icon),
    )?;
    super::symlink(Path::new(&icon), &appdir.join(".DirIcon"))?;
    // The runner resolves its own path through /proc/self/exe, so a link to
    // it starts it with its `lib/` and `data/` found.
    super::symlink(
        Path::new(&format!("{INSTALL}/wellfed")),
        &appdir.join("AppRun"),
    )
}

fn pack_appimage(
    appimagetool: &Path,
    runtime: &Path,
    appdir: &Path,
    appimage: &Path,
    epoch: u64,
) -> Result<(), Error> {
    if appimage.exists() {
        fs::remove_file(appimage).map_err(at(appimage))?;
    }
    status(
        Command::new(appimagetool)
            .arg("--runtime-file")
            .arg(runtime)
            .arg(appdir)
            .arg(appimage)
            // appimagetool is itself an AppImage: this runs it without FUSE.
            .env("APPIMAGE_EXTRACT_AND_RUN", "1")
            .env("ARCH", "x86_64")
            .env("SOURCE_DATE_EPOCH", epoch.to_string()),
    )
}

/// Lists the AppImage's squashfs, which starts where its ELF runtime ends,
/// and checks it holds the app, the bridge and the icon.
fn check_appimage(appimage: &Path) -> Result<(), Error> {
    let bytes = fs::read(appimage).map_err(at(appimage))?;
    let offset = elf_end(&bytes).ok_or_else(|| Error::RuntimeNotElf(appimage.to_path_buf()))?;

    let listed = output(
        Command::new("unsquashfs")
            .args(["-no-progress", "-o", &offset.to_string(), "-l"])
            .arg(appimage),
    )
    .map_err(|error| match error {
        Error::Spawn { .. } => Error::MissingTool {
            tool: "unsquashfs",
            hint: "install squashfs-tools",
        },
        other => other,
    })?;
    let listing: Vec<String> = String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("squashfs-root/"))
        .map(str::to_owned)
        .collect();

    require_listed(
        appimage,
        &listing,
        &[
            "AppRun",
            &format!("{APP_ID}.desktop"),
            &format!("{APP_ID}.png"),
            &format!("{INSTALL}/wellfed"),
            &format!("{INSTALL}/lib/libwellfed_app_bridge.so"),
            &format!("{INSTALL}/lib/libapp.so"),
            &format!("usr/share/icons/hicolor/512x512/apps/{APP_ID}.png"),
        ],
    )
}

/// Where a 64-bit little-endian ELF's section headers end, which is where an
/// AppImage's runtime ends and its squashfs begins.
fn elf_end(bytes: &[u8]) -> Option<u64> {
    let header = bytes.get(..64)?;
    if header.get(..4)? != b"\x7fELF" || header[4] != 2 || header[5] != 1 {
        return None;
    }
    let u64_at = |at: usize| Some(u64::from_le_bytes(header.get(at..at + 8)?.try_into().ok()?));
    let u16_at = |at: usize| Some(u16::from_le_bytes(header.get(at..at + 2)?.try_into().ok()?));
    let (start, size, count) = (u64_at(0x28)?, u16_at(0x3a)?, u16_at(0x3c)?);
    start.checked_add(u64::from(size) * u64::from(count))
}

/// The `.deb`'s two trees: `root/` (what it installs) and `control/`.
fn stage_deb(
    root: &Path,
    version: &Version,
    bundle: &Path,
    stage: &Path,
    epoch: u64,
) -> Result<(), Error> {
    let tree = stage.join("root");
    stage_usr(
        root,
        version,
        bundle,
        &tree,
        &format!("/{INSTALL}/wellfed"),
        epoch,
    )?;

    let license_path = root.join("LICENSE");
    let license = fs::read_to_string(&license_path).map_err(at(&license_path))?;
    let copyright = format!(
        "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/\n\
         Upstream-Name: Wellfed\n\
         Source: https://github.com/Isaac-Duarte/wellfed\n\n\
         Files: *\n\
         Copyright: Wellfed contributors\n\
         License: MIT\n{}",
        license
            .lines()
            .map(|line| if line.trim().is_empty() {
                " .".to_owned()
            } else {
                format!(" {line}")
            })
            .collect::<Vec<_>>()
            .join("\n")
    );
    write(
        &tree.join("usr/share/doc/wellfed/copyright"),
        copyright.as_bytes(),
    )?;

    // Debian's Installed-Size is in KiB.
    let kib = tree_size(&tree)?.div_ceil(1024);
    let control = template(
        root,
        "linux/control",
        &[
            ("VERSION", &version.debian()),
            ("SIZE", &kib.to_string()),
            ("GLIBC", &glibc_floor()),
        ],
    )?;
    write(&stage.join("control/control"), control.as_bytes())?;
    write(&stage.join("control/md5sums"), &md5sums(&tree)?)?;
    write(&stage.join("debian-binary"), b"2.0\n")
}

fn pack_deb(stage: &Path, deb: &Path, epoch: u64) -> Result<(), Error> {
    for (tree, archive) in [("control", "control.tar"), ("root", "data.tar")] {
        status(
            Command::new("tar")
                .args([
                    "--sort=name",
                    &format!("--mtime=@{epoch}"),
                    "--owner=0",
                    "--group=0",
                    "--numeric-owner",
                    "--mode=u+rwX,go+rX,go-w",
                    "--format=gnu",
                    "-cf",
                    archive,
                    "-C",
                    tree,
                    ".",
                ])
                .current_dir(stage),
        )?;
        // One thread, so the stream doesn't depend on the host's cores.
        status(
            Command::new("xz")
                .args(["-T1", "-6", "-f", archive])
                .current_dir(stage),
        )?;
    }

    if deb.exists() {
        fs::remove_file(deb).map_err(at(deb))?;
    }
    status(
        Command::new("ar")
            .arg("rcD")
            .arg(deb)
            .args(["debian-binary", "control.tar.xz", "data.tar.xz"])
            .current_dir(stage),
    )
}

/// Reads the `.deb` back: its members in dpkg's order, its control fields,
/// and that its data holds the app, the bridge and the icon.
fn check_deb(deb: &Path) -> Result<(), Error> {
    let members = output(Command::new("ar").arg("t").arg(deb))?;
    let members: Vec<String> = String::from_utf8_lossy(&members.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    require(
        members == ["debian-binary", "control.tar.xz", "data.tar.xz"],
        || Error::DebMembers {
            package: deb.to_path_buf(),
            members: members.clone(),
        },
    )?;

    let control_xz = output(Command::new("ar").arg("p").arg(deb).arg("control.tar.xz"))?;
    let control = piped(
        &control_xz.stdout,
        Command::new("tar").args(["-xJOf", "-", "./control"]),
    )?;
    let control = String::from_utf8_lossy(&control).into_owned();
    let libc = format!("libc6 (>= {})", glibc_floor());
    for field in [
        "Package: wellfed\n",
        "Architecture: amd64\n",
        "Depends: ",
        &libc,
    ] {
        require(control.contains(field), || Error::MissingEntry {
            package: deb.to_path_buf(),
            entry: format!("the control field {:?}", field.trim()),
        })?;
    }
    let sums = piped(
        &control_xz.stdout,
        Command::new("tar").args(["-xJOf", "-", "./md5sums"]),
    )?;
    require(
        String::from_utf8_lossy(&sums)
            .contains(&format!("  {INSTALL}/lib/libwellfed_app_bridge.so\n")),
        || Error::MissingEntry {
            package: deb.to_path_buf(),
            entry: "the bridge in md5sums".to_owned(),
        },
    )?;

    let data_xz = output(Command::new("ar").arg("p").arg(deb).arg("data.tar.xz"))?;
    let listing = piped(&data_xz.stdout, Command::new("tar").args(["-tJf", "-"]))?;
    let listing: Vec<String> = String::from_utf8_lossy(&listing)
        .lines()
        .map(|line| line.trim_start_matches("./").to_owned())
        .collect();
    require_listed(
        deb,
        &listing,
        &[
            &format!("{INSTALL}/wellfed"),
            &format!("{INSTALL}/lib/libwellfed_app_bridge.so"),
            &format!("{INSTALL}/lib/libapp.so"),
            &format!("usr/share/applications/{APP_ID}.desktop"),
            &format!("usr/share/icons/hicolor/512x512/apps/{APP_ID}.png"),
            "usr/share/doc/wellfed/copyright",
        ],
    )
}

/// Runs a command with `input` on its standard input; its standard output.
fn piped(input: &[u8], command: &mut Command) -> Result<Vec<u8>, Error> {
    let program = command.get_program().to_string_lossy().into_owned();
    let spawn = |source| Error::Spawn {
        program: program.clone(),
        source,
    };
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(spawn)?;
    // The listing is read only after the input is all written, which holds
    // while tar's output fits its pipe; a writer thread keeps that from
    // mattering for a long listing.
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| spawn(std::io::ErrorKind::BrokenPipe.into()))?;
    let input = input.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let out = child.wait_with_output().map_err(spawn)?;
    let written = writer
        .join()
        .map_err(|_| spawn(std::io::ErrorKind::BrokenPipe.into()))?;
    written.map_err(spawn)?;

    require(out.status.success(), || Error::Failed {
        program: program.clone(),
        status: out.status,
        stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
    })?;
    Ok(out.stdout)
}

/// The desktop entry and the AppStream metadata, through their own
/// validators where the host has them. They lint only: a host without them
/// still packs, and says what it didn't check.
fn lint(tree: &Path) -> Result<(), Error> {
    let desktop = tree.join(format!("usr/share/applications/{APP_ID}.desktop"));
    let metainfo = tree.join(format!("usr/share/metainfo/{APP_ID}.appdata.xml"));
    let checks: [(&str, Vec<&std::ffi::OsStr>); 2] = [
        ("desktop-file-validate", vec![desktop.as_os_str()]),
        (
            "appstreamcli",
            vec![
                "validate".as_ref(),
                "--no-net".as_ref(),
                metainfo.as_os_str(),
            ],
        ),
    ];
    for (tool, args) in checks {
        match output(Command::new(tool).args(args)) {
            Ok(_) => {}
            Err(Error::Spawn { .. }) => {
                eprintln!("package-desktop: {tool} is not installed, so it did not check")
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Fails unless a downloaded tool has its pinned SHA-256.
fn require_pinned(path: &Path, pin: Pin) -> Result<(), Error> {
    let bytes = fs::read(path).map_err(at(path))?;
    let found = sha256(&bytes);
    require(found == pin.sha256, || Error::PinMismatch {
        path: path.to_path_buf(),
        found: found.clone(),
        expected: pin.sha256,
        url: pin.url,
    })
}

/// A downloaded tool, by where it comes from and its SHA-256.
#[derive(Clone, Copy, Debug)]
struct Pin {
    url: &'static str,
    sha256: &'static str,
}

/// `2.39`, as the `.deb` writes it.
fn glibc_floor() -> String {
    format!("{}.{}", GLIBC_FLOOR.0, GLIBC_FLOOR.1)
}

/// Fails if the runner or any library in a staged bundle needs a glibc
/// symbol newer than the floor, by the versions `objdump -T` lists.
fn check_glibc(install: &Path) -> Result<(), Error> {
    let mut binaries = vec![install.join("wellfed")];
    let lib = install.join("lib");
    for entry in fs::read_dir(&lib).map_err(at(&lib))? {
        let path = entry.map_err(at(&lib))?.path();
        if path.extension().is_some_and(|ext| ext == "so") {
            binaries.push(path);
        }
    }
    binaries.sort();

    for binary in binaries {
        let symbols =
            output(Command::new("objdump").arg("-T").arg(&binary)).map_err(
                |error| match error {
                    Error::Spawn { .. } => Error::MissingTool {
                        tool: "objdump",
                        hint: "install binutils",
                    },
                    other => other,
                },
            )?;
        let needs = newest_glibc(&String::from_utf8_lossy(&symbols.stdout));
        if let Some(needs) = needs.filter(|needs| *needs > GLIBC_FLOOR) {
            return Err(Error::GlibcTooNew {
                binary,
                needs: format!("{}.{}", needs.0, needs.1),
                floor: glibc_floor(),
            });
        }
    }
    Ok(())
}

/// The newest `GLIBC_x.y[.z]` version in an `objdump -T` listing, as x.y.
fn newest_glibc(listing: &str) -> Option<(u32, u32)> {
    listing
        .split(|c: char| c.is_whitespace() || c == '(' || c == ')')
        .filter_map(|word| word.strip_prefix("GLIBC_"))
        .filter_map(|version| {
            let mut parts = version.split('.');
            Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
        })
        .max()
}

/// The `.deb`'s `md5sums`: every regular file it installs, by its path
/// without a leading `./`, sorted, in `md5sum`'s format.
fn md5sums(tree: &Path) -> Result<Vec<u8>, Error> {
    let mut files = Vec::new();
    regular_files(tree, Path::new(""), &mut files)?;
    files.sort();
    let sums = output(
        Command::new("md5sum")
            .arg("--")
            .args(&files)
            .current_dir(tree),
    )?;
    Ok(sums.stdout)
}

fn regular_files(tree: &Path, relative: &Path, files: &mut Vec<PathBuf>) -> Result<(), Error> {
    let dir = tree.join(relative);
    for entry in fs::read_dir(&dir).map_err(at(&dir))? {
        let entry = entry.map_err(at(&dir))?;
        let kind = entry.file_type().map_err(at(&entry.path()))?;
        let path = relative.join(entry.file_name());
        if kind.is_dir() {
            regular_files(tree, &path, files)?;
        } else if kind.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

/// The total size of a tree's files.
fn tree_size(dir: &Path) -> Result<u64, Error> {
    let mut total = 0;
    for entry in fs::read_dir(dir).map_err(at(dir))? {
        let entry = entry.map_err(at(dir))?;
        let meta = fs::symlink_metadata(entry.path()).map_err(at(&entry.path()))?;
        total += if meta.is_dir() {
            tree_size(&entry.path())?
        } else {
            meta.len()
        };
    }
    Ok(total)
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(at(parent))?;
    }
    fs::write(path, bytes).map_err(at(path))
}

/// A Unix time's UTC date, `YYYY-MM-DD`, for the AppStream release.
fn date_of(epoch: u64) -> String {
    // Howard Hinnant's days-to-civil, on days since 1970-01-01.
    let days = i64::try_from(epoch / 86_400).unwrap_or(0) + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_utc_calendar_days() {
        assert_eq!(date_of(0), "1970-01-01");
        assert_eq!(date_of(951_782_400), "2000-02-29");
        assert_eq!(date_of(1_794_614_400), "2026-11-14");
    }

    #[test]
    fn the_newest_glibc_is_read_from_objdump() {
        let listing = "0000000000000000      DF *UND*\t0000000000000000 (GLIBC_2.2.5) memcpy\n\
                       0000000000000000      DF *UND*\t0000000000000000 (GLIBC_2.39) pidfd_spawnp\n\
                       0000000000000000      DF *UND*\t0000000000000000  GLIBC_2.4   __stack_chk_fail\n";
        assert_eq!(newest_glibc(listing), Some((2, 39)));
        assert_eq!(newest_glibc("no versions here"), None);
        assert!(newest_glibc(listing) <= Some(GLIBC_FLOOR));
    }

    #[test]
    fn the_squashfs_starts_after_the_section_headers() {
        let mut header = vec![0_u8; 64];
        header[..6].copy_from_slice(b"\x7fELF\x02\x01");
        header[0x28..0x30].copy_from_slice(&1000_u64.to_le_bytes());
        header[0x3a..0x3c].copy_from_slice(&64_u16.to_le_bytes());
        header[0x3c..0x3e].copy_from_slice(&3_u16.to_le_bytes());
        assert_eq!(elf_end(&header), Some(1192));

        header[4] = 1;
        assert_eq!(elf_end(&header), None, "a 32-bit runtime");
        assert_eq!(elf_end(b"\x7fELF"), None, "a short file");
    }
}
