//! The source-bound startup identity: a hash of the device, the profile,
//! the firmware and its variant, the provisioning, the update key and every
//! input file's digest.
//!
//! Every Rust-linked image prints it (`PLFW1 start build=…`), the manifest
//! records it, and the bench reuses a frozen review whose identity matches
//! what a build of the current sources would embed. The builder's own
//! sources are inputs, so a changed recipe or audit is a new identity.

use std::collections::BTreeMap;
use std::path::PathBuf;

use wellfed_formats::credentials::{
    KeyScope, OtaKeyFile, RELEASE_KEY_PATH, RELEASE_KEY_VERSION, ReleaseKeyFile,
};
use wellfed_formats::manifest::IdentitySnapshot;
use wellfed_formats::require;
use wellfed_session::hex;

use crate::{
    Device, Firmware, Profile, Provisioning, Repo, Result, Stage, Variant, digest, relative,
};

/// Why the source identity could not be bound.
#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    /// The inputs are listed by Git, so a source tree without it has none.
    #[error("Source identity requires a Git checkout of this repository")]
    NoCheckout,
    #[error("No Rust firmware sources found for the source identity")]
    NoSources,
    #[error("Update key file needs a 64-character lowercase hex public key")]
    PublicKey,
    /// An image embeds only the bench key: an owner's key is trusted by
    /// the feeders that enrolled it, and built in it would also sign key
    /// statements.
    #[error("{} is an owner's update key; images embed only the bench key", path.display())]
    OwnerKey { path: PathBuf },
    /// The release key never sits in a checkout, and a consumer image takes
    /// its public half from [`RELEASE_KEY_PATH`] alone.
    #[error(
        "{} is the release key, which stays offline; consumer images embed it from {RELEASE_KEY_PATH}",
        path.display()
    )]
    ReleaseKey { path: PathBuf },
    /// A consumer image trusts only the release key, never the bench key.
    #[error(
        "A consumer first-boot build needs the release key's public half in {}",
        path.display()
    )]
    NoReleaseKey { path: PathBuf },
    #[error("Release key file version {version} is not {RELEASE_KEY_VERSION}")]
    ReleaseKeyVersion { version: u32 },
}

/// Crates linked into, or testing, every firmware build: the whole SDK and the
/// product crates it links (the device protocol and the product's stores). The
/// selected firmware's directory joins them; other firmwares and host tools
/// such as wellfed-stock and wellfed-flash never change a build's identity.
const SDK_CRATES: [&str; 2] = ["src/sdk", "src/product"];

/// The builder's own sources (its fixture tests are not), plus the
/// wellfed-image modules whose OTA container and region checks the layout
/// and park audits share. The update key a build embeds is bound by its
/// value, so where `wellfed-signing` keeps the key file is not an input.
const BUILDER_SOURCES: &str = "src/tools/wellfed-build/src";
const BUILDER_SHARED: [&str; 3] = [
    "src/tools/wellfed-build/Cargo.toml",
    "src/client/wellfed-image/src/ota.rs",
    "src/client/wellfed-image/src/error.rs",
];

/// Tracked plus untracked, non-ignored files below the root-relative
/// directories `dirs`.
///
/// Ignored files (editor backups, caches, build output) do not change the
/// identity; `target` is excluded only as a directory below the checkout, so
/// the checkout's own location never matters.
fn rust_sources(repo: &Repo, dirs: &[&str]) -> Result<Vec<PathBuf>> {
    let listing = crate::git_command(&repo.root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
        ])
        .args(dirs)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .ok_or(IdentityError::NoCheckout)?
        .stdout;

    let mut paths: Vec<PathBuf> = String::from_utf8_lossy(&listing)
        .split('\0')
        .filter(|name| !name.is_empty())
        .map(|name| repo.root.join(name))
        // Tracked files deleted from the working tree are simply not inputs.
        .filter(|path| {
            path.strip_prefix(&repo.root)
                .is_ok_and(|inside| !inside.iter().any(|part| part == "target"))
                && path.is_file()
        })
        .collect();
    paths.sort();
    paths.dedup();

    require(!paths.is_empty(), || IdentityError::NoSources)?;
    Ok(paths)
}

/// Every file a profile's image depends on, in a stable order.
fn source_inputs(
    repo: &Repo,
    device: &Device,
    profile: Profile,
    provisioning: Provisioning,
    firmware: &Firmware,
) -> Result<Vec<PathBuf>> {
    let mut paths = device
        .platform(repo)
        .inputs(repo, profile, provisioning, firmware)?;
    paths.push(repo.path(&device.config_path()));
    paths.extend(rust_sources(repo, &[BUILDER_SOURCES])?);
    paths.extend(BUILDER_SHARED.map(|p| repo.path(p)));

    if profile.has(Stage::Rust) {
        paths.extend(["Cargo.toml", "Cargo.lock", "rust-toolchain.toml"].map(|p| repo.path(p)));
        let firmware_dir = firmware.dir();
        let mut dirs = SDK_CRATES.to_vec();
        dirs.push(&firmware_dir);
        paths.extend(rust_sources(repo, &dirs)?);
    }
    if profile.has(Stage::Stock) {
        paths.push(device.stock(repo)?.layout);
    }

    let mut seen = std::collections::HashSet::new();
    paths.retain(|path| seen.insert(path.clone()));
    Ok(paths)
}

/// Each input's digest, keyed by its root-relative path as the manifest
/// records it.
fn digests(repo: &Repo, paths: &[PathBuf]) -> Result<BTreeMap<String, String>> {
    paths
        .iter()
        .map(|path| Ok((relative(path, &repo.root)?, digest(path)?)))
        .collect()
}

/// The current inputs of a profile with their digests.
pub fn current_inputs(
    repo: &Repo,
    device: &Device,
    profile: Profile,
    provisioning: Provisioning,
    firmware: &Firmware,
) -> Result<BTreeMap<String, String>> {
    digests(
        repo,
        &source_inputs(repo, device, profile, provisioning, firmware)?,
    )
}

/// The update-signing public key built into a first-boot image: the bench
/// key (None without its file), or for a consumer image the release key,
/// whose file it cannot build without. A consumer build never falls back to
/// the bench key.
///
/// Only a key's public half reaches the build, the identity and the
/// manifest.
pub(crate) fn ota_public_key(
    repo: &Repo,
    profile: Profile,
    provisioning: Provisioning,
) -> Result<Option<String>> {
    if !profile.has(Stage::Firstboot) {
        return Ok(None);
    }

    let public_key = match provisioning {
        Provisioning::Bench => {
            let path = repo.path(wellfed_signing::KEY_PATH);
            if !path.is_file() {
                return Ok(None);
            }
            let key: OtaKeyFile = wellfed_formats::load(&path)?;
            match key.scope {
                KeyScope::Bench => key.public_key,
                KeyScope::Owner => return Err(IdentityError::OwnerKey { path }.into()),
                KeyScope::Release => return Err(IdentityError::ReleaseKey { path }.into()),
            }
        }
        Provisioning::Consumer => {
            let path = repo.path(RELEASE_KEY_PATH);
            require(path.is_file(), || IdentityError::NoReleaseKey {
                path: path.clone(),
            })?;
            let key: ReleaseKeyFile = wellfed_formats::load(&path)?;
            require(key.version == RELEASE_KEY_VERSION, || {
                IdentityError::ReleaseKeyVersion {
                    version: key.version,
                }
            })?;
            key.public_key
        }
    };

    require(
        public_key.len() == 64 && hex::is_lower(public_key.as_bytes()),
        || IdentityError::PublicKey,
    )?;
    Ok(Some(public_key))
}

/// What one build binds into its identity, besides its inputs.
pub(crate) struct Subject<'a> {
    pub(crate) device: &'a Device,
    pub(crate) profile: Profile,
    pub(crate) firmware: &'a Firmware,
    pub(crate) variant: Option<&'a Variant>,
    pub(crate) provisioning: Provisioning,
}

/// Bind the identity to the device, the profile, the firmware a Rust
/// profile links, the variant, the provisioning, the update key and
/// root-relative input digests: SHA-256 of their canonical (sorted, compact)
/// JSON.
pub(crate) fn source_identity(
    subject: &Subject,
    inputs: &BTreeMap<String, String>,
    ota_key: Option<&str>,
) -> Result<String> {
    let snapshot = IdentitySnapshot {
        device: subject.device.name(),
        profile: subject.profile.name(),
        inputs,
        firmware: subject
            .profile
            .has(Stage::Rust)
            .then(|| subject.firmware.name()),
        variant: subject.variant.map(Variant::name),
        provisioning: subject.provisioning,
        ota_public_key: ota_key,
    };
    Ok(snapshot.identity()?)
}

/// Which first-boot identity `source-id` prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceId {
    /// The bare identity of the sources, which check-cross compiles against.
    Bare,
    /// The one a build of this variant would embed (update key included),
    /// which the bench matches reviews by.
    Image,
}

/// The first-boot identity of `firmware` on `device`, provisioned as
/// `provisioning`. A bare identity binds no variant.
pub fn source_id(
    repo: &Repo,
    device: &Device,
    firmware: &Firmware,
    variant: Option<&Variant>,
    provisioning: Provisioning,
    which: SourceId,
) -> Result<String> {
    let profile = Profile::Firstboot;
    let inputs = current_inputs(repo, device, profile, provisioning, firmware)?;
    let mut subject = Subject {
        device,
        profile,
        firmware,
        variant,
        provisioning,
    };
    match which {
        SourceId::Image => {
            let key = ota_public_key(repo, profile, provisioning)?;
            source_identity(&subject, &inputs, key.as_deref())
        }
        SourceId::Bare => {
            subject.variant = None;
            source_identity(&subject, &inputs, None)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::Error;

    const HOTSPOT: &str = "private/hotspot.json";

    /// A checkout with only what `ota_public_key` reads, so no real key file
    /// is needed or touched.
    fn scratch() -> (tempfile::TempDir, Repo) {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("private")).unwrap();
        let repo = Repo {
            root: root.path().to_path_buf(),
        };
        (root, repo)
    }

    fn release_key(repo: &Repo, text: &str) {
        fs::write(repo.path(RELEASE_KEY_PATH), text).unwrap();
    }

    /// The paths a profile's image depends on before the source listing,
    /// root-relative: the platform's and the board's.
    fn platform_inputs(provisioning: Provisioning) -> Vec<String> {
        let repo = Repo::here().unwrap();
        let device = Device::new(&repo, "plaf103").unwrap();
        let firmware = Firmware::new(&repo, "feeder").unwrap();
        device
            .platform(&repo)
            .inputs(&repo, Profile::Firstboot, provisioning, &firmware)
            .unwrap()
            .iter()
            .map(|path| relative(path, &repo.root).unwrap())
            .collect()
    }

    #[test]
    fn a_consumer_image_depends_on_no_build_wifi() {
        let bench = platform_inputs(Provisioning::Bench);
        let consumer = platform_inputs(Provisioning::Consumer);

        assert!(bench.iter().any(|path| path == HOTSPOT));
        assert!(!consumer.iter().any(|path| path == HOTSPOT));
        let without: Vec<&String> = bench.iter().filter(|path| *path != HOTSPOT).collect();
        assert_eq!(without, consumer.iter().collect::<Vec<_>>());
    }

    /// A bench image's inputs are what they were before consumer builds:
    /// the pins, the entry and the hotspot file it is generated from.
    #[test]
    fn the_bench_inputs_keep_the_hotspot_after_the_entry() {
        let bench = platform_inputs(Provisioning::Bench);
        let repo = Repo::here().unwrap();
        let device = Device::new(&repo, "plaf103").unwrap();
        let crate::platform::Platform::Amebad(board) = device.platform(&repo);
        let entry = relative(&board.entry, &repo.root).unwrap();

        assert_eq!(
            bench[..4],
            [
                "src/firmware/vendor/sdk.lock.json",
                "src/firmware/vendor/probe/toolchain.lock.json",
                entry.as_str(),
                HOTSPOT,
            ]
        );
    }

    /// Only the bench key is built in: an owner's key file under the
    /// bench key's path is refused before anything is embedded.
    #[test]
    fn an_image_embeds_the_bench_key_and_never_an_owner_s() {
        let (_root, repo) = scratch();
        let path = repo.path(wellfed_signing::KEY_PATH);
        let bench = Provisioning::Bench;

        let public = wellfed_signing::keygen(&path, KeyScope::Bench).unwrap();
        assert_eq!(
            ota_public_key(&repo, Profile::Firstboot, bench).unwrap(),
            Some(public)
        );

        fs::remove_file(&path).unwrap();
        wellfed_signing::keygen(&path, KeyScope::Owner).unwrap();
        let refused = ota_public_key(&repo, Profile::Firstboot, bench).unwrap_err();
        assert!(
            matches!(&refused, Error::Identity(IdentityError::OwnerKey { path: named }) if *named == path),
            "{refused:?}"
        );
    }

    /// A release key copied into the bench key's place is refused: it
    /// belongs offline, and reaches an image only as its public half.
    #[test]
    fn a_bench_image_refuses_the_release_key_file() {
        let (_root, repo) = scratch();
        let path = repo.path(wellfed_signing::KEY_PATH);
        fs::write(
            &path,
            format!(
                r#"{{"version": 1, "scope": "release", "seed": "{}", "public_key": "{}"}}"#,
                "11".repeat(32),
                "22".repeat(32)
            ),
        )
        .unwrap();

        let refused = ota_public_key(&repo, Profile::Firstboot, Provisioning::Bench).unwrap_err();
        assert!(
            matches!(&refused, Error::Identity(IdentityError::ReleaseKey { path: named }) if *named == path),
            "{refused:?}"
        );
    }

    #[test]
    fn a_consumer_build_trusts_only_the_release_key() {
        let (_root, repo) = scratch();
        let firstboot = Profile::Firstboot;
        fs::write(
            repo.path(wellfed_signing::KEY_PATH),
            format!(
                r#"{{"version": 1, "scope": "bench", "seed": "{}", "public_key": "{}"}}"#,
                "11".repeat(32),
                "22".repeat(32)
            ),
        )
        .unwrap();

        // The bench key is there, but a consumer build refuses without the
        // release key rather than fall back to it.
        let missing = ota_public_key(&repo, firstboot, Provisioning::Consumer);
        assert!(
            matches!(
                missing,
                Err(Error::Identity(IdentityError::NoReleaseKey { ref path }))
                    if path.ends_with("release-key.pub.json")
            ),
            "{missing:?}"
        );
        assert_eq!(
            ota_public_key(&repo, firstboot, Provisioning::Bench).unwrap(),
            Some("22".repeat(32))
        );

        // A synthetic release key: the owner has not generated the real one.
        let release = "ab".repeat(32);
        release_key(
            &repo,
            &format!(r#"{{"version": 1, "public_key": "{release}"}}"#),
        );
        assert_eq!(
            ota_public_key(&repo, firstboot, Provisioning::Consumer).unwrap(),
            Some(release)
        );
        assert_eq!(
            ota_public_key(&repo, Profile::Device, Provisioning::Consumer).unwrap(),
            None,
            "only a first-boot image embeds a key"
        );
    }

    /// The public half `release_keygen` writes, copied in as it is, is the
    /// key a consumer image builds in. That image takes the manifests and
    /// key statements the release key signs, at the installer's check and
    /// through the firmware's own admission, and refuses the bench key's.
    #[test]
    fn a_consumer_image_takes_what_the_release_key_signs_and_not_the_bench_key() {
        use wellfed_core::record::Tag;
        use wellfed_signed::offer::{
            self, Context, KeyError as RingError, KeyRing, KeyStatement, OfferError, StatementError,
        };
        use wellfed_signing::{FrozenError, Key, Motion};

        const BUILD: &str = "8a13e59efa15e56b5f5e32825909dc45e381a2027abc65d8de6658c40b302447";

        let media = tempfile::tempdir().unwrap();
        let release = wellfed_signing::release_keygen(&media.path().join("release.json")).unwrap();
        let (root, repo) = scratch();
        fs::copy(&release.public, repo.path(RELEASE_KEY_PATH)).unwrap();
        let bench_path = repo.path(wellfed_signing::KEY_PATH);
        wellfed_signing::keygen(&bench_path, KeyScope::Bench).unwrap();
        let (release_key, bench_key) = (
            Key::load(&release.private).unwrap(),
            Key::load(&bench_path).unwrap(),
        );

        let built_in = ota_public_key(&repo, Profile::Firstboot, Provisioning::Consumer)
            .unwrap()
            .unwrap();
        assert_eq!(built_in, release.public_key);

        // A frozen consumer build that records the key, as a real build's
        // manifest does.
        let image = wellfed_image::fixtures::image_carrying(
            format!("{} start build={BUILD} bank=", Tag::Start).as_bytes(),
        );
        let review = root.path().join("private/review");
        let fields = serde_json::json!({"startup_identity": BUILD, "firmware": "feeder",
                                        "provisioning": "consumer", "ota_public_key": built_in});
        wellfed_image::fixtures::freeze_build(root.path(), &review, &image, &fields).unwrap();
        let frozen = wellfed_signing::Frozen::open(root.path(), &review).unwrap();
        let by_release = frozen.offer(&release_key, 9, Motion::Refused).unwrap();
        let by_bench = frozen.offer(&bench_key, 9, Motion::Refused).unwrap();

        frozen.require_installable(&by_release).unwrap();
        let refused = frozen.require_installable(&by_bench).unwrap_err();
        assert!(
            matches!(
                refused,
                wellfed_signing::Error::Frozen(FrozenError::NotBuiltInSigner { .. })
            ),
            "{refused:?}"
        );

        let built_in = [hex::decode_array::<32>(built_in.as_bytes()).unwrap()];
        let ring = KeyRing {
            built_in: &built_in,
            added: None,
            enrolled: &[],
            revoked: &[],
        };
        let context = Context {
            model: wellfed_proto::device::Model::Plaf103.code(),
            ring,
            floor: 0,
            device: [0x42; 32],
            running: None,
        };
        assert!(offer::admit_offer(&by_release, None, &context).is_ok());
        assert_eq!(
            offer::admit_offer(&by_bench, None, &context),
            Err(OfferError::Key(RingError::Unknown))
        );

        let added = [0x08; 32];
        assert_eq!(
            offer::admit_statement(&release_key.add_key(&added).unwrap(), &ring),
            Ok(KeyStatement::Add(added))
        );
        assert_eq!(
            offer::admit_statement(&release_key.revoke(&bench_key.id()).unwrap(), &ring),
            Ok(KeyStatement::Revoke(bench_key.id()))
        );
        for statement in [
            bench_key.add_key(&added).unwrap(),
            bench_key.revoke(&release_key.id()).unwrap(),
        ] {
            assert_eq!(
                offer::admit_statement(&statement, &ring),
                Err(StatementError::Key(RingError::Unknown))
            );
        }
    }

    #[test]
    fn a_bad_release_key_file_is_refused() {
        let (_root, repo) = scratch();
        let consumer = |text: &str| {
            release_key(&repo, text);
            ota_public_key(&repo, Profile::Firstboot, Provisioning::Consumer)
        };

        assert!(matches!(
            consumer(&format!(
                r#"{{"version": 2, "public_key": "{}"}}"#,
                "ab".repeat(32)
            )),
            Err(Error::Identity(IdentityError::ReleaseKeyVersion {
                version: 2
            }))
        ));
        assert!(matches!(
            consumer(&format!(
                r#"{{"version": 1, "public_key": "{}"}}"#,
                "AB".repeat(32)
            )),
            Err(Error::Identity(IdentityError::PublicKey))
        ));
        // The private half never enters a build.
        assert!(
            consumer(&format!(
                r#"{{"version": 1, "seed": "{}", "public_key": "{}"}}"#,
                "cd".repeat(32),
                "ab".repeat(32)
            ))
            .is_err()
        );
    }

    #[test]
    fn the_identity_binds_the_provisioning_and_the_release_key() {
        let repo = Repo::here().unwrap();
        let device = Device::new(&repo, "plaf103").unwrap();
        let firmware = Firmware::new(&repo, "feeder").unwrap();
        let inputs = BTreeMap::from([("a".to_string(), "00".repeat(32))]);
        let subject = |provisioning| Subject {
            device: &device,
            profile: Profile::Firstboot,
            firmware: &firmware,
            variant: None,
            provisioning,
        };
        let id = |provisioning, key: &str| {
            source_identity(&subject(provisioning), &inputs, Some(key)).unwrap()
        };

        let (one, two) = ("ab".repeat(32), "cd".repeat(32));
        assert_ne!(
            id(Provisioning::Bench, &one),
            id(Provisioning::Consumer, &one)
        );
        assert_ne!(
            id(Provisioning::Consumer, &one),
            id(Provisioning::Consumer, &two)
        );
    }
}
