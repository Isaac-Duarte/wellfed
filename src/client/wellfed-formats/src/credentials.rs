//! The private credential and key files: the hotspot's Wi-Fi credentials,
//! the bench update key `wellfed-build keygen` writes and the release key's
//! public half a consumer build embeds, which `wellfed-build` turns into
//! headers and build settings, an owner's update key
//! `wellfed update-key new` writes, which a feeder trusts only once the
//! owner enrolls it, and the project release key
//! `wellfed-build release-keygen` writes onto offline media.

use serde::{Deserialize, Serialize};

/// The development Wi-Fi network the first-boot image joins. Its
/// `Debug` leaves the password out, so no error or log line carries it.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct WifiCredential {
    pub ssid: String,
    pub password: String,
}

impl std::fmt::Debug for WifiCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WifiCredential")
            .field("ssid", &self.ssid)
            .finish_non_exhaustive()
    }
}

/// An update-signing key; `seed` never leaves the file. The bench key, an
/// owner's key and the release key share the format: each is an Ed25519
/// seed that signs the same version 2 manifests, and they differ only in how
/// a feeder comes to trust them, which `scope` names.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct OtaKeyFile {
    pub version: u32,
    /// What the key may sign.
    pub scope: KeyScope,
    /// Ed25519 seed, hex.
    pub seed: String,
    /// Hex; the firmware build embeds it.
    pub public_key: String,
}

impl std::fmt::Debug for OtaKeyFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OtaKeyFile")
            .field("version", &self.version)
            .field("scope", &self.scope)
            .field("public_key", &self.public_key)
            .finish_non_exhaustive()
    }
}

/// What an update-signing key is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyScope {
    /// Development images on the bench's own units. It lives in a
    /// checkout's `private/`, and the firmware builds embed its public half.
    Bench,
    /// An owner's own builds, for the feeders that enrolled it as their
    /// update signer (a Dispense press at each feeder confirms it). No
    /// image embeds it, and a feeder that did not enroll it refuses what it
    /// signs.
    Owner,
    /// The project release key, kept offline and never in a checkout. A
    /// consumer image embeds its public half from [`RELEASE_KEY_PATH`],
    /// never from this file, and it signs manifests and key statements as
    /// any built-in key does.
    Release,
}

/// The project release key's public half, `private/release-key.pub.json`,
/// which a consumer build embeds as its built-in update key.
///
/// The release key itself ([`KeyScope::Release`]) is kept offline and never
/// becomes a file in a checkout; `wellfed-build release-keygen` writes this
/// file beside it, to be copied in. Unknown fields are refused, so a file that also carries a
/// `seed` (an [`OtaKeyFile`] copied by mistake) never feeds a build.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseKeyFile {
    /// [`RELEASE_KEY_VERSION`].
    pub version: u32,
    /// Ed25519 public key, 64 lowercase hex characters.
    pub public_key: String,
}

/// The one [`ReleaseKeyFile`] version.
pub const RELEASE_KEY_VERSION: u32 = 1;

/// Where a checkout keeps the [`ReleaseKeyFile`], root-relative.
pub const RELEASE_KEY_PATH: &str = "private/release-key.pub.json";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_key_file_holds_only_the_public_half() {
        let public = "ab".repeat(32);
        let file: ReleaseKeyFile =
            serde_json::from_str(&format!(r#"{{"version": 1, "public_key": "{public}"}}"#))
                .unwrap();
        assert_eq!(file.public_key, public);

        let with_seed = format!(
            r#"{{"version": 1, "seed": "{}", "public_key": "{public}"}}"#,
            "cd".repeat(32)
        );
        assert!(serde_json::from_str::<ReleaseKeyFile>(&with_seed).is_err());
    }

    #[test]
    fn an_update_key_s_debug_names_the_public_key_but_not_the_seed() {
        let key = OtaKeyFile {
            version: 1,
            scope: KeyScope::Owner,
            seed: "5eed5eed".into(),
            public_key: "9ab1c".into(),
        };
        let debug = format!("{key:?}");
        assert!(debug.contains("9ab1c"), "{debug}");
        assert!(!debug.contains("5eed"), "{debug}");
    }

    #[test]
    fn a_wifi_credential_s_debug_names_the_network_but_not_its_password() {
        let credential = WifiCredential {
            ssid: "Bench".into(),
            password: "hunter2 secret".into(),
        };
        let debug = format!("{credential:?}");
        assert!(debug.contains("Bench"), "{debug}");
        assert!(!debug.contains("hunter2"), "{debug}");
    }
}
