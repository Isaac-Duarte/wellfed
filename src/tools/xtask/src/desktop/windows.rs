//! Windows: an Inno Setup installer, per user and unsigned.
//!
//! The staged folder is the runner's Release folder plus the Visual C++
//! runtime's DLLs from the build machine's Visual Studio, since the runner
//! and the Rust bridge both load them and a clean Windows may not have them
//! (an app-local copy is how Microsoft lets an app carry them).
//!
//! `check-windows-installer` installs the result for real, checks what it
//! put where, uninstalls it and checks the app's stores are still there. It
//! refuses on a computer where Wellfed is already installed, since the
//! uninstall would remove that one.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use wellfed_formats::require;

use super::{
    APP, Error, Options, Version, at, copy_tree, flutter_build, output, require_built, status,
};

/// The release build, relative to the app.
const RELEASE: &str = "build/windows/x64/runner/Release";
/// What the release build must hold. The Inno Setup script names the same
/// files, so its build fails if the staged folder lacks one.
const BUILT: &[&str] = &[
    "wellfed.exe",
    "wellfed_app_bridge.dll",
    "flutter_windows.dll",
    "data/app.so",
    "data/icudtl.dat",
    "data/flutter_assets",
];
/// The script's `AppId`, as Windows keys the uninstall entry.
const UNINSTALL_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\{24CF1924-B47F-475A-9CB4-20CE6E0FE06E}_is1";
/// How long the uninstaller, which hands itself to a copy in the temporary
/// folder and returns at once, may take to finish.
const UNINSTALL_WAIT: Duration = Duration::from_secs(120);
/// What the installer check leaves in a store folder that wasn't there.
const MARKER: &str = "installer-check";

pub(super) fn package(
    root: &Path,
    version: &Version,
    options: &Options,
    stage: &Path,
    out: &Path,
) -> Result<PathBuf, Error> {
    let iscc = match &options.iscc {
        Some(iscc) => iscc.clone(),
        None => find_iscc()?,
    };
    if options.build == super::Build::Fresh {
        flutter_build(root, "windows")?;
    }
    let release = root.join(APP).join(RELEASE);
    require_built(&release, BUILT)?;

    let staged = stage.join("windows");
    copy_tree(&release, &staged)?;
    for dll in runtime_dlls()? {
        let name = dll.file_name().map(PathBuf::from).unwrap_or_default();
        fs::copy(&dll, staged.join(name)).map_err(at(&dll))?;
    }

    // Inno Setup reads a relative path from the script's own folder.
    let absolute = |path: &Path| std::path::absolute(path).map_err(at(path));
    let name = format!("Wellfed-{}-windows-setup", version.name);
    status(
        Command::new(iscc)
            .arg("/Q")
            .arg(format!("/DAppVersion={}", version.name))
            .arg(format!("/DFileVersion={}", version.windows_file()))
            .arg(format!("/DSourceDir={}", absolute(&staged)?.display()))
            .arg(format!("/DOutputDir={}", absolute(out)?.display()))
            .arg(format!("/DOutputName={name}"))
            .arg(absolute(
                &root.join(super::PACKAGING).join("windows/wellfed.iss"),
            )?),
    )?;
    Ok(out.join(format!("{name}.exe")))
}

/// Inno Setup's compiler where winget or its own installer puts it.
fn find_iscc() -> Result<PathBuf, Error> {
    let candidates = [
        std::env::var_os("LOCALAPPDATA")
            .map(|dir| PathBuf::from(dir).join(r"Programs\Inno Setup 6\ISCC.exe")),
        std::env::var_os("ProgramFiles(x86)")
            .map(|dir| PathBuf::from(dir).join(r"Inno Setup 6\ISCC.exe")),
        std::env::var_os("ProgramFiles")
            .map(|dir| PathBuf::from(dir).join(r"Inno Setup 6\ISCC.exe")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|path| path.exists())
        .ok_or(Error::MissingTool {
            tool: "Inno Setup 6",
            hint: "winget install -e --id JRSoftware.InnoSetup, or pass --iscc",
        })
}

/// The newest x64 Visual C++ runtime DLLs of the Visual Studio that has the
/// C++ tools, which is what built the runner and the bridge.
fn runtime_dlls() -> Result<Vec<PathBuf>, Error> {
    let missing = || Error::MissingTool {
        tool: "the Visual C++ redistributable DLLs",
        hint: "install Visual Studio's C++ build tools",
    };
    let vswhere = std::env::var_os("ProgramFiles(x86)")
        .map(|dir| PathBuf::from(dir).join(r"Microsoft Visual Studio\Installer\vswhere.exe"))
        .ok_or_else(missing)?;
    let found = output(Command::new(vswhere).args([
        "-latest",
        "-products",
        "*",
        "-requires",
        "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
        "-property",
        "installationPath",
    ]))?;
    let install = PathBuf::from(String::from_utf8_lossy(&found.stdout).trim());
    let redist = install.join(r"VC\Redist\MSVC");

    // Version folders sort by their numbers; the newest with an x64 CRT wins.
    let mut versions: Vec<(Vec<u32>, PathBuf)> = Vec::new();
    for entry in fs::read_dir(&redist).map_err(at(&redist))? {
        let path = entry.map_err(at(&redist))?.path();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let numbers: Option<Vec<u32>> = name.split('.').map(|part| part.parse().ok()).collect();
        if let Some(numbers) = numbers {
            versions.push((numbers, path));
        }
    }
    versions.sort();
    for (_, version) in versions.iter().rev() {
        let x64 = version.join("x64");
        let Ok(entries) = fs::read_dir(&x64) else {
            continue;
        };
        let crt = entries.flatten().map(|entry| entry.path()).find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".CRT"))
        });
        if let Some(crt) = crt {
            let mut dlls = Vec::new();
            for entry in fs::read_dir(&crt).map_err(at(&crt))? {
                let path = entry.map_err(at(&crt))?.path();
                if path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("dll"))
                {
                    dlls.push(path);
                }
            }
            require(!dlls.is_empty(), missing)?;
            dlls.sort();
            return Ok(dlls);
        }
    }
    Err(missing())
}

/// Installs `setup` silently, checks the install, uninstalls it, and checks
/// the stores survived.
pub(crate) fn check_installer(setup: &Path) -> Result<(), Error> {
    if !cfg!(windows) {
        return Err(Error::UnsupportedHost(std::env::consts::OS));
    }
    let local = env_dir("LOCALAPPDATA")?;
    let roaming = env_dir("APPDATA")?;
    let installed = local.join(r"Programs\Wellfed");
    let shortcut = roaming.join(r"Microsoft\Windows\Start Menu\Programs\Wellfed.lnk");
    require(!registered() && !installed.exists(), || {
        Error::AlreadyInstalled(installed.clone())
    })?;
    // The app's stores, and the client secret, which path_provider keeps in
    // the roaming folder named from the runner's company and product.
    let stores = [
        local.join("Wellfed"),
        roaming.join(r"Wellfed contributors\Wellfed"),
    ];

    // The stores must outlast an uninstall; where there are none yet, a
    // marker stands in.
    let mut kept = Vec::new();
    for store in &stores {
        let marked = !store.exists();
        if marked {
            fs::create_dir_all(store).map_err(at(store))?;
            let marker = store.join(MARKER);
            fs::write(&marker, b"left by cargo xtask check-windows-installer\n")
                .map_err(at(&marker))?;
        }
        kept.push((store, marked, names(store)?));
    }

    let checked = install_and_remove(setup, &installed, &shortcut);

    let mut survived = Ok(());
    for (store, marked, before) in kept {
        let after = names(store);
        if marked {
            // Only what this check made goes; anything the app wrote stays.
            let _ = fs::remove_file(store.join(MARKER));
            let _ = fs::remove_dir(store);
        }
        let whole = after.is_ok_and(|after| before.iter().all(|name| after.contains(name)));
        if survived.is_ok() && !whole {
            survived = Err(Error::DataRemoved(store.clone()));
        }
    }
    checked?;
    survived
}

fn install_and_remove(setup: &Path, installed: &Path, shortcut: &Path) -> Result<(), Error> {
    status(Command::new(setup).args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-"]))?;

    require(registered(), || Error::MissingEntry {
        package: setup.to_path_buf(),
        entry: format!("the uninstall entry {UNINSTALL_KEY}"),
    })?;
    for file in [
        "wellfed.exe",
        "wellfed_app_bridge.dll",
        "flutter_windows.dll",
        "vcruntime140.dll",
        "msvcp140.dll",
        r"data\app.so",
        r"data\icudtl.dat",
        "unins000.exe",
    ] {
        let path = installed.join(file);
        require(path.exists(), || Error::MissingEntry {
            package: setup.to_path_buf(),
            entry: path.display().to_string(),
        })?;
    }
    require(shortcut.exists(), || Error::MissingEntry {
        package: setup.to_path_buf(),
        entry: shortcut.display().to_string(),
    })?;

    let uninstaller = installed.join("unins000.exe");
    status(Command::new(&uninstaller).args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"]))?;
    let started = Instant::now();
    while registered() || installed.exists() || shortcut.exists() {
        require(started.elapsed() < UNINSTALL_WAIT, || {
            Error::NotRemoved(installed.to_path_buf())
        })?;
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(())
}

/// Whether Windows lists the app as installed for this user.
fn registered() -> bool {
    Command::new("reg")
        .args(["query", UNINSTALL_KEY])
        .output()
        .is_ok_and(|out| out.status.success())
}

fn env_dir(name: &'static str) -> Result<PathBuf, Error> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or(Error::NoEnvironment(name))
}

fn names(dir: &Path) -> Result<Vec<String>, Error> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir).map_err(at(dir))? {
        names.push(
            entry
                .map_err(at(dir))?
                .file_name()
                .to_string_lossy()
                .into_owned(),
        );
    }
    Ok(names)
}
