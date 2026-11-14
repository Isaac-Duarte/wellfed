//! The folder that holds the client secret on Windows, held to its name.
//!
//! On Windows, `flutter_secure_storage` keeps the client secret in
//! `%APPDATA%\<CompanyName>\<ProductName>\flutter_secure_storage.dat`,
//! encrypted with DPAPI for the user, and path_provider takes both names from
//! the runner's version resource (`windows/runner/Runner.rc`). Renaming
//! either would point an updated app at an empty folder: it would make a new
//! key no feeder knows, and the owner would take every feeder back through
//! Recover a feeder at its panel. The other stores are in
//! `%LOCALAPPDATA%\Wellfed`, which the app names itself (`platform/host.dart`).
//!
//! A deliberate rename needs a move of the old file first, which no build
//! does yet; change these strings only together with one.

use std::path::PathBuf;

/// What the version resource must keep saying.
const FIXED: [(&str, &str); 2] = [
    ("CompanyName", "Wellfed contributors"),
    ("ProductName", "Wellfed"),
];

#[test]
fn the_windows_secret_folder_keeps_its_name() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../windows/runner/Runner.rc");
    let resource = std::fs::read_to_string(&path).expect("the Windows runner's resource script");

    for (field, value) in FIXED {
        let line = format!("VALUE \"{field}\", \"{value}\" \"\\0\"");
        let found: Vec<&str> = resource
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with(&format!("VALUE \"{field}\"")))
            .collect();
        assert_eq!(
            found,
            [line.as_str()],
            "Runner.rc's {field} names the folder holding the Windows client secret; \
             renaming it strands every installed app's key (see this test's notes)"
        );
    }
}
