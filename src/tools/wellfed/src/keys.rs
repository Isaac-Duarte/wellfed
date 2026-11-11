//! The owner's client key file (`wellfed_formats::owner`): created once
//! from the system's random source, only by a command that pairs or joins
//! and was told to (`--create-key`), then reused for every feeder. Any
//! other open of a missing file is refused with the path it looked at, so a
//! command pointed at the wrong place never makes a second key that no
//! feeder knows.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use wellfed_formats::owner::{self, EnrolledFeeder, OwnerKeyFile, VERSION};
use wellfed_formats::private::{self, SYSTEM_RANDOM};
use wellfed_session::hex;
use wellfed_session::noise::Keypair;

use crate::home::HOME_VAR;

/// What an open does when there is no key file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Missing {
    /// Make a new key there.
    Create,
    /// Refuse, naming the path.
    Refuse,
}

impl Missing {
    /// A command's `--create-key`.
    pub(crate) const fn when(create: bool) -> Self {
        if create { Self::Create } else { Self::Refuse }
    }
}

/// Why the key file did not open.
#[derive(Debug, thiserror::Error)]
pub(crate) enum KeyError {
    #[error(
        "no owner key at {}: pass --keys or set {HOME_VAR} to where yours is, or run setup, recover or join with --create-key to make one there",
        path.display()
    )]
    Missing { path: PathBuf },
}

/// The label a new key file gets unless `--label` names one.
pub(crate) const DEFAULT_LABEL: &str = "wellfed-cli";

pub(crate) struct OwnerKey {
    path: PathBuf,
    file: OwnerKeyFile,
    keypair: Keypair,
}

impl OwnerKey {
    /// The key at `path`; with [`Missing::Create`], a new one written there
    /// when there is none. `label` replaces the stored label when given.
    pub(crate) fn open(
        path: &Path,
        label: Option<&str>,
        missing: Missing,
        random: &Path,
    ) -> Result<Self> {
        let exists = path
            .try_exists()
            .with_context(|| format!("look for {}", path.display()))?;
        match (exists, missing) {
            (true, _) => Self::existing(path, label),
            (false, Missing::Create) => Self::create(path, label, random),
            (false, Missing::Refuse) => Err(KeyError::Missing {
                path: path.to_path_buf(),
            }
            .into()),
        }
    }

    /// The key file at `path`, relabelled when `label` differs.
    fn existing(path: &Path, label: Option<&str>) -> Result<Self> {
        let file =
            owner::load(path).with_context(|| format!("read the key file {}", path.display()))?;
        let mut key = Self::of(path, file)?;
        // Every command opens the key: rewrite it only when it changed.
        if let Some(label) = label.filter(|label| *label != key.file.label) {
            key.file.label = label.into();
            key.save()?;
        }
        Ok(key)
    }

    /// A new key at `path`. The file appears whole, and only where none is:
    /// two runs that both found no key race to write it, and the one that
    /// loses opens the winner's rather than replace a key a feeder may
    /// already have enrolled.
    fn create(path: &Path, label: Option<&str>, random: &Path) -> Result<Self> {
        let secret: [u8; 32] = private::random(random).context("read the system random source")?;
        let file = OwnerKeyFile {
            version: VERSION,
            secret: hex::Lower(&secret).to_string(),
            label: label.unwrap_or(DEFAULT_LABEL).into(),
            feeders: Vec::new(),
        };
        let mut bytes = serde_json::to_vec_pretty(&file).context("encode the key file")?;
        bytes.push(b'\n');

        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        match private::write_new_whole(path, &bytes) {
            Ok(()) => Self::of(path, file),
            Err(wellfed_formats::Error::Io(error))
                if error.kind() == std::io::ErrorKind::AlreadyExists =>
            {
                Self::existing(path, label)
            }
            Err(error) => {
                Err(error).with_context(|| format!("write the key file {}", path.display()))
            }
        }
    }

    fn of(path: &Path, file: OwnerKeyFile) -> Result<Self> {
        let secret = hex::decode_array_either_case::<32>(file.secret.as_bytes())
            .with_context(|| format!("{}: the secret is not 32 bytes of hex", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            keypair: Keypair::from_secret(secret),
            file,
        })
    }

    /// The default random source.
    pub(crate) fn system() -> &'static Path {
        Path::new(SYSTEM_RANDOM)
    }

    pub(crate) fn keypair(&self) -> &Keypair {
        &self.keypair
    }

    pub(crate) fn label(&self) -> &str {
        &self.file.label
    }

    /// The feeder to talk to: the one whose identity starts with `prefix`,
    /// or the only one this key knows.
    pub(crate) fn feeder(&self, prefix: Option<&str>) -> Result<[u8; 32]> {
        let matching: Vec<&EnrolledFeeder> = self
            .file
            .feeders
            .iter()
            .filter(|f| prefix.is_none_or(|p| f.identity.starts_with(p)))
            .collect();
        let [feeder] = matching.as_slice() else {
            anyhow::bail!(
                "{}: {} feeders match; pass --feeder with a prefix of one",
                self.path.display(),
                matching.len()
            );
        };
        hex::decode_array_either_case::<32>(feeder.identity.as_bytes())
            .context("a feeder identity in the key file")
    }

    /// Note a feeder that enrolled this key.
    pub(crate) fn remember(&mut self, identity: &[u8; 32]) -> Result<()> {
        let identity = hex::Lower(identity).to_string();
        if self.file.feeders.iter().any(|f| f.identity == identity) {
            return Ok(());
        }
        self.file.feeders.push(EnrolledFeeder { identity });
        self.save()
    }

    fn save(&self) -> Result<()> {
        owner::save(&self.path, &self.file)
            .with_context(|| format!("write the key file {}", self.path.display()))
    }
}

/// The key file is the driver's client key: what it opens sessions and
/// setups with, and where a feeder that enrolled it is noted.
impl wellfed_install::Keys for OwnerKey {
    fn keypair(&self) -> &Keypair {
        Self::keypair(self)
    }

    fn label(&self) -> &str {
        Self::label(self)
    }

    fn feeder(&self, prefix: Option<&str>) -> Option<[u8; 32]> {
        Self::feeder(self, prefix).ok()
    }

    fn remember(&mut self, identity: &[u8; 32]) -> Result<(), wellfed_install::PlatformError> {
        Ok(Self::remember(self, identity)?)
    }
}

/// The identity's first 8 bytes, as the feeder logs them (`PLID1`).
pub(crate) fn fingerprint(identity: &[u8; 32]) -> String {
    hex::Lower(&identity[..8]).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_created_once_and_remembers_its_feeders() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("owner.json");
        let random = dir.path().join("random");
        std::fs::write(&random, [9u8; 32]).unwrap();

        let Err(refused) = OwnerKey::open(&path, None, Missing::Refuse, &random) else {
            panic!("a missing key is refused");
        };
        assert!(matches!(
            refused.downcast_ref::<KeyError>(),
            Some(KeyError::Missing { path: looked }) if *looked == path
        ));
        assert!(!path.exists(), "a refused open makes nothing");

        let mut key = OwnerKey::open(&path, None, Missing::Create, &random).unwrap();
        assert_eq!(key.label(), DEFAULT_LABEL);
        let public = *key.keypair().public();
        key.remember(&[5; 32]).unwrap();
        key.remember(&[5; 32]).unwrap();

        std::fs::write(&random, [1u8; 32]).unwrap();
        let again = OwnerKey::open(&path, Some("laptop"), Missing::Create, &random).unwrap();
        assert_eq!(*again.keypair().public(), public, "the same key");
        assert_eq!(again.label(), "laptop");
        assert_eq!(again.file.feeders.len(), 1);
        assert_eq!(fingerprint(&[0xab; 32]), "abababababababab");

        // Opening a key that does not change leaves its file alone.
        let modified = || std::fs::metadata(&path).unwrap().modified().unwrap();
        let before = modified();
        let mut same = OwnerKey::open(&path, Some("laptop"), Missing::Create, &random).unwrap();
        same.remember(&[5; 32]).unwrap();
        assert_eq!(modified(), before);
    }

    /// Two runs that both found no key: the one that writes second opens
    /// the first one's key and leaves its file as it was.
    #[test]
    fn a_key_made_by_a_concurrent_run_is_opened_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("home").join("owner.json");
        let random = dir.path().join("random");
        std::fs::write(&random, [9u8; 32]).unwrap();
        let first = OwnerKey::create(&path, None, &random).unwrap();
        let written = std::fs::read(&path).unwrap();

        std::fs::write(&random, [1u8; 32]).unwrap();
        let second = OwnerKey::create(&path, None, &random).unwrap();

        assert_eq!(second.keypair().public(), first.keypair().public());
        assert_eq!(std::fs::read(&path).unwrap(), written, "not replaced");
        let left: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, ["owner.json"], "no temporary file left");
    }

    #[test]
    fn a_damaged_key_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("owner.json");
        std::fs::write(
            &path,
            r#"{"version":1,"secret":"00","label":"x","feeders":[]}"#,
        )
        .unwrap();
        assert!(OwnerKey::open(&path, None, Missing::Create, &path).is_err());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            br#"{"version":1,"secret":"00","label":"x","feeders":[]}"#,
            "a key file that does not open is never replaced"
        );
    }
}
