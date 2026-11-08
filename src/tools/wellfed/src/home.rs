//! Where the CLI keeps the owner's files by default: the owner's client key
//! (`wellfed-owner.json`), the stock backups (`stock-backups/`) and the
//! server login (`wellfed-server.json`), all in one directory.
//!
//! That directory is `WELLFED_HOME` when it is set, and otherwise the
//! platform's data directory for the app: `$XDG_DATA_HOME/wellfed` (or
//! `~/.local/share/wellfed`) on Linux, Application Support on macOS and
//! AppData on Windows. It never depends on the working directory, so a
//! command run from anywhere finds the same key. Each file's flag (`--keys`,
//! `--backups`, `--token-file`) still names another path. The names inside
//! are the ones `private/` has always used, so `WELLFED_HOME=private` in a
//! checkout keeps the bench's layout.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use etcetera::AppStrategy;
use etcetera::app_strategy::{AppStrategyArgs, choose_app_strategy};

/// The environment variable that names the directory.
pub(crate) const HOME_VAR: &str = "WELLFED_HOME";

/// Why there is no directory to default to.
#[derive(Debug, thiserror::Error)]
pub(crate) enum HomeError {
    #[error("no data directory for wellfed: set {HOME_VAR}, or pass the file's flag ({0})")]
    Platform(#[from] etcetera::HomeDirError),
}

/// The directory the owner's files default to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Home(PathBuf);

impl Home {
    /// `WELLFED_HOME`, or the platform's data directory.
    pub(crate) fn locate() -> Result<Self, HomeError> {
        Self::from(std::env::var_os(HOME_VAR), platform)
    }

    /// `set` when it names a directory, else what `platform` finds.
    fn from(
        set: Option<OsString>,
        platform: impl FnOnce() -> Result<PathBuf, HomeError>,
    ) -> Result<Self, HomeError> {
        match set.filter(|set| !set.is_empty()) {
            Some(set) => Ok(Self(PathBuf::from(set))),
            None => Ok(Self(platform()?)),
        }
    }

    pub(crate) fn owner_key(&self) -> PathBuf {
        self.0.join("wellfed-owner.json")
    }

    pub(crate) fn backups(&self) -> PathBuf {
        self.0.join("stock-backups")
    }

    pub(crate) fn server_token(&self) -> PathBuf {
        self.0.join("wellfed-server.json")
    }
}

fn platform() -> Result<PathBuf, HomeError> {
    let strategy = choose_app_strategy(AppStrategyArgs {
        top_level_domain: "io.github".into(),
        author: "isaacduarte".into(),
        app_name: "wellfed".into(),
    })?;
    Ok(strategy.data_dir())
}

/// `given`, or the file `pick` names in the [`Home`].
pub(crate) fn or_home(
    given: Option<&Path>,
    pick: impl FnOnce(&Home) -> PathBuf,
) -> Result<PathBuf, HomeError> {
    match given {
        Some(given) => Ok(given.to_path_buf()),
        None => Ok(pick(&Home::locate()?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_variable_names_the_directory_and_the_platform_is_the_fallback() {
        let platform = || Ok(PathBuf::from("/data/wellfed"));
        let set = Home::from(Some("/elsewhere".into()), || unreachable!()).unwrap();
        assert_eq!(set.owner_key(), Path::new("/elsewhere/wellfed-owner.json"));
        assert_eq!(set.backups(), Path::new("/elsewhere/stock-backups"));
        assert_eq!(
            set.server_token(),
            Path::new("/elsewhere/wellfed-server.json")
        );

        for unset in [None, Some(OsString::new())] {
            assert_eq!(
                Home::from(unset, platform).unwrap().owner_key(),
                Path::new("/data/wellfed/wellfed-owner.json")
            );
        }
    }

    #[test]
    fn private_as_the_home_keeps_the_checkout_s_layout() {
        let home = Home::from(Some("private".into()), || unreachable!()).unwrap();
        assert_eq!(home.owner_key(), Path::new("private/wellfed-owner.json"));
        assert_eq!(home.backups(), Path::new("private/stock-backups"));
        assert_eq!(
            home.server_token(),
            Path::new("private/wellfed-server.json")
        );
    }

    #[test]
    fn a_flag_wins_over_the_home() {
        let given = Path::new("/my/key.json");
        assert_eq!(or_home(Some(given), |_| unreachable!()).unwrap(), given);
    }

    /// The platform's directory on this host: an absolute path ending in
    /// the app's name, whatever the working directory.
    #[test]
    fn the_platform_directory_is_absolute() {
        let path = platform().unwrap();
        assert!(path.is_absolute(), "{}", path.display());
        assert!(path.ends_with("wellfed") || path.to_string_lossy().contains("wellfed"));
    }
}
