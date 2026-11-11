//! The project release key, which `wellfed-build release-keygen` writes once
//! onto offline media.
//!
//! Every consumer image trusts it, and bank 0 keeps the build it was written
//! with, so a lost key strands every beta feeder on its current firmware
//! until someone reinstalls over UART, and a leaked one lets anyone sign
//! for them. The command therefore never writes into a Git working tree,
//! where a careless `git add` or a synced checkout could carry the seed off,
//! and never replaces a key. Beside the key it writes the public half in
//! exactly the [`ReleaseKeyFile`] shape, the one file that is copied into a
//! checkout's `private/`.

use std::path::{Path, PathBuf};

use wellfed_formats::credentials::{KeyScope, RELEASE_KEY_VERSION, ReleaseKeyFile};
use wellfed_formats::private::write_new_whole;
use wellfed_formats::require;
use wellfed_signed::offer::KeyId;

use crate::{Key, KeyError, Result, write_key};

/// What [`release_keygen`] wrote.
#[derive(Debug)]
pub struct ReleaseKey {
    /// The key itself, `"scope": "release"`, mode 600.
    pub private: PathBuf,
    /// `<stem>.pub.json` beside it: the [`ReleaseKeyFile`] to copy into a
    /// checkout as `private/release-key.pub.json`.
    pub public: PathBuf,
    /// The public key, 64 lowercase hex characters.
    pub public_key: String,
    /// The id a manifest names the key by.
    pub id: KeyId,
}

/// Create the release key at `output`, an absolute path outside any Git
/// working tree, and its public half beside it. Refuses to replace either
/// file.
pub fn release_keygen(output: &Path) -> Result<ReleaseKey> {
    require(output.is_absolute(), || KeyError::RelativePath {
        path: output.to_path_buf(),
    })?;
    let public = public_path(output)?;
    require_outside_checkout(output)?;
    for path in [output, public.as_path()] {
        // A dangling symlink counts too: writing through it would land
        // wherever it points.
        require(path.symlink_metadata().is_err(), || KeyError::Exists {
            path: path.to_path_buf(),
        })?;
    }

    let public_key = write_key(output, KeyScope::Release)?;
    let file = ReleaseKeyFile {
        version: RELEASE_KEY_VERSION,
        public_key: public_key.clone(),
    };
    let mut text = serde_json::to_vec_pretty(&file)?;
    text.push(b'\n');
    if let Err(error) = write_new_whole(&public, &text) {
        // This run created the key a moment ago and has shown nobody its
        // id. Left alone, a rerun would meet "already exists" for a key with
        // no public half, and the owner could not tell whether it was ever
        // copied. A failed removal leaves the write error to report anyway.
        let _removed = std::fs::remove_file(output);
        return Err(error.into());
    }

    // Read back as the signer will, so a key that would not load is caught
    // before the owner copies it anywhere.
    let id = Key::load(output)?.id();
    Ok(ReleaseKey {
        private: output.to_path_buf(),
        public,
        public_key,
        id,
    })
}

/// `<stem>.pub.json` beside `output`.
fn public_path(output: &Path) -> Result<PathBuf> {
    let stem = output.file_stem().ok_or_else(|| KeyError::NoFileName {
        path: output.to_path_buf(),
    })?;
    let mut name = stem.to_owned();
    name.push(".pub.json");
    Ok(output.with_file_name(name))
}

/// Refuse `output` if it, as written or once its directory's symlinks are
/// resolved, lies inside a Git working tree: a directory with a `.git`
/// entry, which a linked worktree's is a file.
pub(crate) fn require_outside_checkout(output: &Path) -> Result<()> {
    let directory = output.parent().ok_or_else(|| KeyError::NoFileName {
        path: output.to_path_buf(),
    })?;
    let resolved = std::fs::canonicalize(directory).map_err(|source| KeyError::NoDirectory {
        path: directory.to_path_buf(),
        source,
    })?;

    let checkout = [directory, resolved.as_path()]
        .into_iter()
        .flat_map(Path::ancestors)
        .find(|ancestor| ancestor.join(".git").symlink_metadata().is_ok());
    match checkout {
        Some(checkout) => Err(KeyError::InCheckout {
            path: output.to_path_buf(),
            checkout: checkout.to_path_buf(),
        }
        .into()),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use wellfed_formats::credentials::OtaKeyFile;
    use wellfed_session::hex;

    use super::*;
    use crate::Error;

    #[test]
    fn the_release_key_and_its_public_half_are_written_beside_each_other() {
        let media = tempfile::tempdir().unwrap();
        let output = media.path().join("wellfed-release-key.json");

        let written = release_keygen(&output).unwrap();

        assert_eq!(written.private, output);
        assert_eq!(
            written.public,
            media.path().join("wellfed-release-key.pub.json")
        );
        let file: OtaKeyFile = wellfed_formats::load(&output).unwrap();
        assert_eq!(file.scope, KeyScope::Release);
        assert_eq!(file.public_key, written.public_key);
        assert_eq!(
            fs::metadata(&output).unwrap().permissions().mode() & 0o777,
            0o600
        );

        // Exactly the file a consumer build reads, which refuses any other
        // field, so the seed cannot ride along.
        let public: ReleaseKeyFile = wellfed_formats::load(&written.public).unwrap();
        assert_eq!(
            public,
            ReleaseKeyFile {
                version: RELEASE_KEY_VERSION,
                public_key: written.public_key.clone(),
            }
        );

        let key = Key::load(&output).unwrap();
        assert_eq!(key.scope(), KeyScope::Release);
        assert_eq!(key.id(), written.id);
        assert_eq!(hex::Lower(&key.public()).to_string(), written.public_key);
    }

    #[test]
    fn an_existing_key_or_public_half_is_never_replaced() {
        let media = tempfile::tempdir().unwrap();
        let output = media.path().join("release.json");
        release_keygen(&output).unwrap();
        let before = fs::read(&output).unwrap();

        let refused = release_keygen(&output).unwrap_err();
        assert!(
            matches!(&refused, Error::Key(KeyError::Exists { path }) if *path == output),
            "{refused:?}"
        );
        assert_eq!(fs::read(&output).unwrap(), before);

        // Only the public half is there: still refused, and no key written.
        let other = media.path().join("other.json");
        fs::write(media.path().join("other.pub.json"), b"{}").unwrap();
        let refused = release_keygen(&other).unwrap_err();
        assert!(
            matches!(&refused, Error::Key(KeyError::Exists { path }) if path.ends_with("other.pub.json")),
            "{refused:?}"
        );
        assert!(!other.exists());
    }

    #[test]
    fn a_relative_path_is_refused() {
        let refused = release_keygen(Path::new("release.json")).unwrap_err();
        assert!(
            matches!(refused, Error::Key(KeyError::RelativePath { .. })),
            "{refused:?}"
        );
    }

    #[test]
    fn a_path_inside_a_git_working_tree_is_refused() {
        let media = tempfile::tempdir().unwrap();
        let nested = media.path().join("checkout/private/keys");
        fs::create_dir_all(&nested).unwrap();

        // A checkout's `.git` is a directory, a linked worktree's a file.
        for git_is_directory in [true, false] {
            let marker = media.path().join("checkout/.git");
            if git_is_directory {
                fs::create_dir(&marker).unwrap();
            } else {
                fs::write(&marker, b"gitdir: elsewhere\n").unwrap();
            }

            let output = nested.join("release.json");
            let refused = release_keygen(&output).unwrap_err();
            assert!(
                matches!(&refused, Error::Key(KeyError::InCheckout { checkout, .. })
                    if *checkout == media.path().join("checkout")),
                "{refused:?}"
            );
            assert!(fs::read_dir(&nested).unwrap().next().is_none());

            if git_is_directory {
                fs::remove_dir(&marker).unwrap();
            } else {
                fs::remove_file(&marker).unwrap();
            }
        }

        // Reached through a symlink from outside, it is still the checkout.
        fs::create_dir(media.path().join("checkout/.git")).unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link = outside.path().join("link");
        std::os::unix::fs::symlink(&nested, &link).unwrap();
        let refused = release_keygen(&link.join("release.json")).unwrap_err();
        assert!(
            matches!(refused, Error::Key(KeyError::InCheckout { .. })),
            "{refused:?}"
        );
    }

    #[test]
    fn a_missing_directory_is_refused() {
        let media = tempfile::tempdir().unwrap();
        let refused = release_keygen(&media.path().join("absent/release.json")).unwrap_err();
        assert!(
            matches!(refused, Error::Key(KeyError::NoDirectory { .. })),
            "{refused:?}"
        );
    }
}
