# Desktop packages

This folder holds what turns the app's release builds into installers for macOS, Linux and Windows, and this page says how to build and check them. It is for whoever cuts a release. The installers are unsigned; the [owner guide](../../../docs/guide/before-you-start.md#get-the-app-on-a-computer) says how each system opens them the first time. Building one touches no feeder.

| System | Artifact | Holds |
| --- | --- | --- |
| macOS 12 or later, Apple silicon and Intel | `Wellfed-<version>-macos.dmg` | `Wellfed.app`, signed ad hoc as Flutter builds it, and a link to Applications |
| Linux x86_64 | `Wellfed-<version>-x86_64.AppImage` | The release bundle, a desktop entry and the icon |
| Debian 13, Ubuntu 24.04 and later | `wellfed_<version>_amd64.deb` | The bundle in `/usr/lib/wellfed/`, a desktop entry, the icon and AppStream metadata |
| Windows 10 1809 or later, 64-bit | `Wellfed-<version>-windows-setup.exe` | An Inno Setup installer for the current user, into `%LOCALAPPDATA%\Programs\Wellfed`, with a Start menu shortcut and an uninstaller |

The version is the one in `src/app/pubspec.yaml`, without its build number. The `wellfed` command line is a developer tool and none of these packages carry it. Nothing goes in `/usr/bin` either, on purpose: a `wellfed` there would shadow or be shadowed by the developer CLI of the same name, so the desktop entry starts `/usr/lib/wellfed/wellfed` by its full path.

## What is here

| File | Is |
| --- | --- |
| `linux/io.github.isaacduarte.wellfed.desktop` | The desktop entry, named by the app's id so a Wayland compositor matches its window to it |
| `linux/io.github.isaacduarte.wellfed.appdata.xml` | The AppStream metadata software centers read |
| `linux/control` | The `.deb`'s control file |
| `windows/wellfed.iss` | The Inno Setup script |

`cargo xtask package-desktop` (`src/tools/xtask/src/desktop/`) fills in their version fields, stages the release build, packs it and checks the result.

## Build them

Each system's package is built on that system, since Flutter can't cross-build a desktop. Every run writes into `dist/desktop/<version>/` at the repository root, which Git ignores, and rewrites `SHA256SUMS` over every artifact in that folder. `--skip-build` packs the release build already in the app's `build` folder instead of building one.

### Linux

You need the [Linux build's toolchain](../README.md#toolchain), plus `tar`, `xz`, `ar` (binutils), `curl` and `unsquashfs` (squashfs-tools). No root and no `dpkg`.

1. Run `make appimage-tools` once. It downloads appimagetool 1.9.1 and the AppImage runtime of release 20251108 into `~/.cache/wellfed/appimage` (set `APPIMAGE_TOOLS` to put them elsewhere), and checks each against the SHA-256 pinned in `src/tools/xtask/src/desktop/linux.rs`, which the Makefile reads. The packager checks both hashes again before it runs appimagetool or embeds the runtime.
2. Run `make desktop-packages`. You should see `Built …AppImage` and `Built …deb`.

`desktop-file-validate` and `appstreamcli` check the desktop entry and the metadata where they are installed, and the run says when one isn't.

### macOS

You need the [macOS build's toolchain](../README.md#toolchain) and Rust. Then run `make desktop-packages`, or `cargo xtask package-desktop`. You should see `Built …dmg`.

### Windows

You need Flutter, Rust and Visual Studio's C++ build tools, as for `flutter build windows`, and Inno Setup 6:

```powershell
winget install --source winget -e --id JRSoftware.InnoSetup
```

1. Run `cargo xtask package-desktop` from the repository root. It finds `ISCC.exe` where winget or Inno Setup's own installer puts it, or takes `--iscc`. You should see `Built …windows-setup.exe`.
2. Run `cargo xtask check-windows-installer dist\desktop\<version>\Wellfed-<version>-windows-setup.exe`. It installs the app for real, checks the files, the Start menu shortcut and the uninstall entry, uninstalls it, and checks the app's stores are still there. It refuses on a computer where Wellfed is already installed, since its uninstall would remove that one.

### All of them together

Copy the Mac's and Windows's artifacts into one `dist/desktop/<version>/`, then run `cargo xtask desktop-sums` there. It rewrites `SHA256SUMS` over the four.

## What each check proves

| Package | Checked |
| --- | --- |
| Release build, every system | The runner, the Rust bridge, Dart's compiled code and the engine are there before anything is staged |
| Linux bundle | No binary in it needs a glibc symbol newer than the floor in `linux.rs` (2.39), by what `objdump -T` lists, checked in both staged trees before packing. The `.deb`'s `libc6 (>= …)` comes from the same constant |
| AppImage | Its squashfs, listed with `unsquashfs` from where the runtime ends, holds `AppRun`, the desktop entry, the icon, the runner, `libwellfed_app_bridge.so` and `libapp.so` |
| `.deb` | Its three members are in dpkg's order, its control names the package, its architecture, its dependencies and the glibc floor, its `md5sums` lists the bridge, and its data holds the runner, the bridge, `libapp.so`, the desktop entry, the icon and the copyright file |
| Disk image | `hdiutil verify` passes. Mounted read-only, it holds the runner, the bridge's framework, `App.framework`, `AppIcon.icns` and the Applications link. `codesign --verify --deep --strict` passes on the app, and the runner and the bridge each hold both arm64 and x86_64 |
| Windows installer | Inno Setup names the runner, the bridge, the engine, the C++ runtime and Dart's code one by one, so its build fails if one is missing. `check-windows-installer` then installs and uninstalls it |

A package that fails a check fails the run.

## What the packages need from the system

The Linux packages carry the release bundle and nothing else, and use the system's libraries. The `.deb` names them in `Depends`, from what the bundle links (`objdump -p`) and what Flutter's engine opens at run time (EGL and GLES, which no header names). A bare container with only those installed starts the app as far as its keyring.

| Needs | For |
| --- | --- |
| glibc 2.39 or later | The build host's glibc sets it: the Rust bridge needs 2.39 when built on the Arch builder (the runner 2.34, the secure storage plugin 2.38). A build that needs more fails rather than raise the floor silently |
| GTK 3, GLib, fontconfig, libepoxy, EGL and GLES | Flutter's Linux embedding |
| libsecret, and a running Secret Service (GNOME Keyring, KWallet or KeePassXC) | The client secret. With none, the app stops at start and says so. The `.deb` recommends `gnome-keyring \| kwallet6 \| libkf5wallet-bin \| keepassxc`: any one installed satisfies it, so a KDE desktop with KWallet pulls in nothing, and only a system with none gets GNOME Keyring |
| libdbus | The bridge's Bluetooth, through BlueZ |
| NetworkManager or `iw` (recommended) | Reading the Wi-Fi's name during an install |

`libdartjni.so` in the bundle names `libjvm.so`. Only Android loads it, so no package depends on a JVM.

The Windows installer carries the Visual C++ runtime's DLLs beside the app, from the build machine's Visual Studio, since the runner and the bridge both load them.

On Windows, `flutter_secure_storage` keeps the client secret in `%APPDATA%\Wellfed contributors\Wellfed`, encrypted with DPAPI for the user, apart from the other stores in `%LOCALAPPDATA%\Wellfed`. That folder roams with a roaming profile, and path_provider names it from `Runner.rc`'s CompanyName and ProductName, so renaming either would strand every installed app's key. The bridge's `tests/windows_store.rs` fails if either changes. Moving the secret beside the other stores waits for a maintained, safe Rust wrapper for DPAPI: the one on crates.io sits on the unmaintained `winapi` crate.

## What is reproducible

- **Linux.** Packing the same release bundle twice gives the same bytes: every file gets the last commit's time, owner root and fixed modes, `xz` runs on one thread, `ar` in its deterministic mode, and appimagetool takes `SOURCE_DATE_EPOCH`. Two clean `flutter build linux --release` runs on one host, with sccache serving the Rust, gave the same bundle. A third, from an earlier commit with the same app sources, differed only in the Rust bridge; the cause is not isolated, and a build without the cache has not been compared.
- **Windows.** Packing the same build twice gave the same installer.
- **macOS.** Not reproducible: `hdiutil` writes new times and ids into every image, and the ad hoc signature covers the build's own.
- Builds on different hosts or paths are not expected to match: the bridge's compiler embeds paths, and nothing pins the C compilers or the SDKs.

## Evidence

- **Observed, November 13, 2026.** All four packages built and passed their checks from one commit: Linux on an Arch host, the disk image on a Mac, the installer in a Windows 11 virtual machine. In clean Debian 13 and Ubuntu 24.04 containers, the `.deb` installed with apt resolving its dependencies, and both it and the AppImage started under Xvfb (the AppImage to its first screen) once a keyring was running. In the virtual machine the installer installed silently, the installed app started to its first screen, and the uninstaller removed the app, its shortcut and its uninstall entry and left both store folders. No real Mac's Gatekeeper, real PC's SmartScreen or Linux desktop session has opened a downloaded package yet.
