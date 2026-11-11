//! Update signing, only ever of a frozen review: `wellfed-build keygen`
//! and `wellfed-build sign`, and the statements the device API's update
//! path takes (`downgrade`, `revoke`, `add-key`). The bench links it to sign
//! its offers without linking the builder.
//!
//! The firmware accepts any image whose manifest the embedded key signed,
//! so the signer is the last place a reviewed build can be swapped for an
//! unreviewed one. [`Frozen::open`] therefore verifies a review directory
//! exactly as `wellfed-flash` does before it installs one
//! (`wellfed_image::review::frozen`): the image, payload and
//! manifest hashes, the pinned stock hash for a stock review, the first-boot
//! profile and its recorded SDK and toolchain pins (checked against the lock
//! at freeze time, not the checkout's current one), the manifest's
//! `OTA_All.bin`, and the
//! recorded audits. It also requires the image to carry the manifest's
//! startup identity as text. The build ID and the motion policy come from
//! that verified manifest, never from the caller, and [`Frozen::offer`]
//! refuses a motion image unless the caller names the exact policy it is
//! attending.
//!
//! A key is the bench key in `private/ota-key.json` ([`KeyScope::Bench`]),
//! whose public half the bench builds embed, an owner's own key
//! ([`KeyScope::Owner`]), which a feeder trusts only once the owner has
//! enrolled it there with a Dispense press, or the project release key
//! ([`KeyScope::Release`]), whose public half consumer builds embed from
//! `private/release-key.pub.json`. All three are the same file, but the
//! release key signs only a consumer build with no variant that builds it
//! back in, since every consumer feeder trusts it. The release key is
//! written once onto offline media by
//! [`release_keygen`], never into a checkout, and is loaded from there only
//! to sign.
//!
//! The manifest bytes are `wellfed_signed::offer`'s version 2 format,
//! which the firmware verifies and the device API takes: it names its key
//! and its image kind.
//!
//! A statement binds what it allows: a downgrade names the feeder's identity
//! key, the exact manifest and the floor it was made at; a key statement
//! names the key it retires or adds. Only a key built into the firmware may
//! sign a key statement, and the bench and release keys are the ones built
//! in: an owner's key signs manifests and nothing else a feeder accepts.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use wellfed_signed::offer::{self, ImageKind as OfferKind, KeyId, Signed};

use wellfed_formats::bench::VERIFICATION;
use wellfed_formats::canonical::sha256;
use wellfed_formats::credentials::{KeyScope, OtaKeyFile};
use wellfed_formats::manifest::{BuildManifest, Provisioning};
use wellfed_formats::private::{SYSTEM_RANDOM, random, write_new_whole};
use wellfed_formats::require;
use wellfed_formats::review::ImageKind;
use wellfed_image::bank::BANK1;
use wellfed_image::ota::STOCK_SHA256;
use wellfed_image::review::frozen;
use wellfed_image::root::Root;
use wellfed_proto::device::Model;
use wellfed_session::hex;
use wellfed_signed::{MARKER, Manifest};

mod error;
mod release;

pub use error::{Error, FrozenError, ImageError, KeyError, Result};
pub use release::{ReleaseKey, release_keygen};

fn require_built_in_signer(built_in: &str, build_id: &[u8; 32], manifest: &[u8]) -> Result<()> {
    let signed = offer::parse_manifest(manifest).map_err(Error::NotManifest)?;
    require(signed.manifest.build_id == *build_id, || {
        FrozenError::OtherBuild {
            manifest: hex::Lower(&signed.manifest.build_id).to_string(),
            review: hex::Lower(build_id).to_string(),
        }
    })?;

    let public = id_bytes(built_in)?;
    require(signed.key_id == offer::key_id(&public), || {
        FrozenError::NotBuiltInSigner {
            signer: hex::Lower(&signed.key_id).to_string(),
            built_in: hex::Lower(&offer::key_id(&public)).to_string(),
        }
    })?;
    Ok(())
}

/// The bench key's root-relative path. Firmware builds embed its public half.
pub const KEY_PATH: &str = "private/ota-key.json";

/// The highest version signed so far in the checkout at `root`, with its
/// file: the bench's manifests in [`VERIFICATION`] and the promotions
/// a run kept beside each feeder's backup under `backups`
/// (root-relative unless absolute; `wellfed_formats::bench::STOCK_BACKUPS`
/// by default). The bench
/// and the installer both sign one above it, so neither reuses a version
/// the other signed.
pub fn signed_so_far(root: &Path, backups: &Path) -> Option<(u32, PathBuf)> {
    let mut directories = vec![root.join(VERIFICATION)];
    if let Ok(entries) = std::fs::read_dir(root.join(backups)) {
        directories.extend(entries.flatten().map(|entry| entry.path()));
    }
    directories
        .iter()
        .filter_map(|directory| highest_signed(directory))
        .max_by_key(|(version, _)| *version)
}

/// The highest version among the signed manifests (`*.manifest`) in
/// `directory`, with its file; `None` when it holds none. Only the header
/// is read, unverified: a feeder checks the signature.
pub fn highest_signed(directory: &Path) -> Option<(u32, PathBuf)> {
    std::fs::read_dir(directory)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "manifest"))
        .filter_map(|path| {
            Some((
                wellfed_signed::claimed_version(&std::fs::read(&path).ok()?)?,
                path,
            ))
        })
        .max_by_key(|(version, _)| *version)
}

/// The OTA container's header precedes the bank contents.
pub const CONTAINER_HEADER: usize = 32;

/// Create a new bench or owner key at `output`; refuses to replace one.
/// Returns the public key (hex): firmware builds embed a bench key's, and an
/// owner enrolls theirs on each feeder. The release key is refused here:
/// only [`release_keygen`] writes one, since only it keeps the key out of a
/// checkout.
pub fn keygen(output: &Path, scope: KeyScope) -> Result<String> {
    match scope {
        KeyScope::Bench | KeyScope::Owner => write_key(output, scope),
        KeyScope::Release => Err(KeyError::ReleaseKeygen.into()),
    }
}

/// Write a new key of any scope at `output`, whole, mode 600, never
/// replacing one; the public key, as hex.
fn write_key(output: &Path, scope: KeyScope) -> Result<String> {
    let seed: [u8; 32] = random(Path::new(SYSTEM_RANDOM))?;
    let public_key =
        hex::Lower(&SigningKey::from_bytes(&seed).verifying_key().to_bytes()).to_string();

    let file = OtaKeyFile {
        version: 1,
        scope,
        seed: hex::Lower(&seed).to_string(),
        public_key: public_key.clone(),
    };
    let mut text = serde_json::to_vec_pretty(&file)?;
    text.push(b'\n');
    // Whole or not at all: a partial file would refuse every later load.
    write_new_whole(output, &text)?;

    Ok(public_key)
}

/// A loaded signing key, and what its file says it is for.
pub struct Key {
    key: SigningKey,
    scope: KeyScope,
}

impl Key {
    /// Load a key file, refusing one whose public key is not its seed's.
    pub fn load(path: &Path) -> Result<Self> {
        let file: OtaKeyFile = wellfed_formats::load(path)?;
        require(file.version == 1, || KeyError::Version {
            version: file.version,
        })?;
        let seed =
            hex::decode_array_either_case::<32>(file.seed.as_bytes()).ok_or(KeyError::Seed)?;

        let key = SigningKey::from_bytes(&seed);
        require(
            hex::Lower(&key.verifying_key().to_bytes()).to_string() == file.public_key,
            || KeyError::PublicKey,
        )?;
        Ok(Self {
            key,
            scope: file.scope,
        })
    }

    /// Load a release-scope key from offline media, refusing a file inside
    /// a Git working tree as `release_keygen` refuses to write one there,
    /// or any other scope. A bench stand-in made with `keygen` in a
    /// checkout's `private/` fails both: it is never a release key's
    /// successor.
    pub fn load_release(path: &Path) -> Result<Self> {
        release::require_outside_checkout(path)?;
        let key = Self::load(path)?;
        require(key.scope == KeyScope::Release, || KeyError::NotRelease {
            path: path.to_path_buf(),
            scope: key.scope,
        })?;
        Ok(key)
    }

    /// The public half, as the firmware embeds it or an owner enrolls it.
    pub fn public(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }

    /// What the key file says the key is for.
    pub fn scope(&self) -> KeyScope {
        self.scope
    }

    /// The id a version 2 manifest names this key by.
    pub fn id(&self) -> KeyId {
        offer::key_id(&self.public())
    }

    fn signed<const N: usize>(&self, body: &[u8; N]) -> Vec<u8> {
        let mut out = body.to_vec();
        out.extend_from_slice(&self.key.sign(body).to_bytes());
        out
    }

    /// A downgrade statement: the feeder whose identity key is `device` may
    /// install `manifest` while its floor is `floor`. Any trusted key signs
    /// one, an owner's enrolled key included: the firmware checks a
    /// downgrade's signer as it checks a manifest's, and the statement is
    /// bound to one feeder, one manifest and one floor.
    pub fn downgrade(&self, manifest: &[u8], device: &[u8; 32], floor: u32) -> Result<Vec<u8>> {
        offer::parse_manifest(manifest).map_err(Error::NotManifest)?;
        let digest = offer::manifest_digest(manifest);
        Ok(self.signed(&offer::downgrade_body(&self.id(), device, &digest, floor)))
    }

    /// A key statement retiring `id` for good. Only a key an image builds
    /// in signs one, the bench or the release key: a feeder refuses an
    /// owner's, so one is never written.
    ///
    /// A key never signs its own retirement. Where another key may still
    /// sign statements, that key signs it; where none may, the feeder would
    /// be left with no key that can sign one, and on a consumer feeder
    /// with no key for releases either. The statement names no feeder, so
    /// its bytes would do the same to every feeder they reach.
    pub fn revoke(&self, id: &KeyId) -> Result<Vec<u8>> {
        self.require_built_in(KeyError::OwnerRevoke)?;
        require(*id != self.id(), || KeyError::SelfRevoke)?;
        Ok(self.signed(&offer::revoke_body(&self.id(), id)))
    }

    /// A key statement filling the release slot with `public`, signed only
    /// by a key an image builds in, as for [`Self::revoke`].
    ///
    /// The release key adds only itself. Every consumer feeder trusts it,
    /// and the statement names no feeder, so adding another key would put
    /// that key in the empty slot of every consumer feeder the bytes reach.
    /// A new release key arrives in a build that builds it in, which then
    /// adds itself ([`Frozen::hand_over`]).
    pub fn add_key(&self, public: &[u8; 32]) -> Result<Vec<u8>> {
        self.require_built_in(KeyError::OwnerAddKey)?;
        require(
            self.scope != KeyScope::Release || *public == self.public(),
            || KeyError::ReleaseAddsOther {
                added: hex::Lower(&offer::key_id(public)).to_string(),
            },
        )?;
        Ok(self.signed(&offer::key_add_body(&self.id(), public)))
    }

    /// Refuse, as `refusal`, a key no image builds in.
    fn require_built_in(&self, refusal: KeyError) -> Result<()> {
        match self.scope {
            KeyScope::Bench | KeyScope::Release => Ok(()),
            KeyScope::Owner => Err(refusal.into()),
        }
    }
}

/// Whoever signs a motion image is attending it, naming the policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion<'a> {
    /// Sign only an image that moves nothing at startup.
    Refused,
    /// Also sign an image with exactly this startup motion policy.
    Attended(&'a str),
}

/// A review directory, verified as `wellfed-flash` verifies one.
#[derive(Debug)]
pub struct Frozen {
    image: Vec<u8>,
    kind: ImageKind,
    build_id: [u8; 32],
    manifest: Option<BuildManifest>,
}

impl Frozen {
    /// Open `review`, a directory under `root`'s `private/`, refusing one
    /// without `review.json` or that differs from what was frozen.
    pub fn open(root: &Path, review: &Path) -> Result<Self> {
        require(review.join("review.json").is_file(), || {
            FrozenError::NoReview {
                review: review.to_path_buf(),
            }
        })?;
        let verified = frozen(&Root::new(root)?, review, &BANK1)?;
        let payload = verified.image.get(CONTAINER_HEADER..).unwrap_or_default();

        let kind = verified.review.image_kind;
        let build_id = match (kind, &verified.manifest) {
            // Stock (the pinned image only) has no startup identity: its
            // payload hash names it, and can never equal a build's identity.
            (ImageKind::Stock, None) => id_bytes(&verified.review.payload_sha256)?,
            (ImageKind::Build, Some(manifest)) => {
                let identity = manifest.startup_identity.as_deref().unwrap_or_default();
                let build_id = id_bytes(identity)?;
                // Every Rust-linked image prints its identity at startup.
                require(contains(payload, identity.as_bytes()), || {
                    FrozenError::NoIdentity {
                        identity: identity.to_owned(),
                    }
                })?;
                build_id
            }
            _ => return Err(FrozenError::KindMismatch.into()),
        };

        Ok(Self {
            image: verified.image,
            kind,
            build_id,
            manifest: verified.manifest,
        })
    }

    /// Refuse a `manifest` an installer could not rely on for this image.
    /// An installer carries its manifest from before the feeder has any
    /// enrolled key to trust, so only the image's built-in key can sign it.
    /// The firmware's own admission then checks it against that key alone
    /// (the signature, the model, the bank and the size bound), and it must
    /// name this image's kind and length: the feeder finds a bad manifest
    /// only at promotion, after the install's earlier steps ran.
    pub fn require_installable(&self, manifest: &[u8]) -> Result<()> {
        self.require_same_image(manifest)?;

        let built_in = self
            .manifest
            .as_ref()
            .and_then(|manifest| manifest.ota_public_key.as_deref())
            .ok_or(FrozenError::NoBuiltInKey)?;
        require_built_in_signer(built_in, &self.build_id, manifest)?;

        let keys = [id_bytes(built_in)?];
        let context = offer::Context {
            ring: offer::KeyRing {
                built_in: &keys,
                added: None,
                enrolled: &[],
                revoked: &[],
            },
            floor: 0,
            device: [0; 32],
            running: None,
            model: self.model()?.code(),
        };
        let admitted = offer::admit_offer(manifest, None, &context)
            .map_err(|error| FrozenError::NotAdmitted { error })?;
        let signed = admitted.signed;
        require(signed.kind == OfferKind::Replacement, || {
            FrozenError::OtherKind
        })?;
        let length = self.payload().len();
        require(
            usize::try_from(signed.manifest.image_len).is_ok_and(|bytes| bytes == length),
            || {
                FrozenError::OtherLength {
                    manifest: signed.manifest.image_len,
                    image: length,
                }
                .into()
            },
        )
    }

    /// Refuse a `manifest` signed over other bytes than this review's image.
    /// Two freezes of one build share its identity but not always its bytes,
    /// and the feeder checks the hash only after writing every sector over
    /// bank 1, which may hold the only copy of stock on the feeder.
    pub fn require_same_image(&self, manifest: &[u8]) -> Result<()> {
        let signed = offer::parse_manifest(manifest).map_err(Error::NotManifest)?;
        let image: [u8; 32] = Sha256::digest(self.payload()).into();
        require(signed.manifest.image_sha256 == image, || {
            FrozenError::OtherImage {
                manifest: hex::Lower(&signed.manifest.image_sha256).to_string(),
                review: hex::Lower(&image).to_string(),
            }
            .into()
        })
    }

    /// The pinned stock image as given, without a review directory: an
    /// owner's own backup, for "Restore stock". A stock review binds
    /// nothing but the image, and only the pinned container passes here,
    /// as there; its payload hash names it.
    pub fn stock(image: Vec<u8>) -> Result<Self> {
        require(sha256(&image) == STOCK_SHA256, || FrozenError::NotStock)?;
        let payload = wellfed_image::bank::payload(&image, &BANK1)?;
        let build_id = Sha256::digest(payload).into();
        Ok(Self {
            image,
            kind: ImageKind::Stock,
            build_id,
            manifest: None,
        })
    }

    /// [`Self::stock`] without the pin, for tests and simulators, which
    /// cannot hold the vendor's image. Never in a build that ships: only
    /// the `fixtures` feature has it.
    #[cfg(feature = "fixtures")]
    pub fn synthetic_stock(image: Vec<u8>) -> Self {
        let build_id = Sha256::digest(image.get(CONTAINER_HEADER..).unwrap_or_default()).into();
        Self {
            image,
            kind: ImageKind::Stock,
            build_id,
            manifest: None,
        }
    }

    /// Stock or a build, as the verified review records it.
    pub fn kind(&self) -> ImageKind {
        self.kind
    }

    /// The device model the image is built for, which its manifest names:
    /// a build's own, as its frozen build manifest records it, and for the
    /// pinned stock image the PLAF103 (the pin is its stock). A device no
    /// model has is refused, and so is any model but the PLAF103, whose
    /// frozen `plaf103_*` review kinds are the only ones this signs.
    pub fn model(&self) -> Result<Model> {
        let Some(manifest) = &self.manifest else {
            return Ok(Model::Plaf103);
        };
        let model = Model::ALL
            .into_iter()
            .find(|model| model.name() == manifest.device)
            .ok_or_else(|| FrozenError::UnknownDevice {
                device: manifest.device.clone(),
            })?;
        require(model == Model::Plaf103, || FrozenError::OtherModel {
            model,
        })?;
        Ok(model)
    }

    /// The build ID the manifest will carry, as lowercase hex.
    pub fn build_id(&self) -> String {
        hex::Lower(&self.build_id).to_string()
    }

    /// A build's frozen manifest; `None` for stock.
    pub fn manifest(&self) -> Option<&BuildManifest> {
        self.manifest.as_ref()
    }

    /// The startup motion policy the frozen build declares, if it moves.
    pub fn motion_policy(&self) -> Option<&str> {
        self.manifest
            .as_ref()
            .and_then(|m| m.startup_motion_policy.as_deref())
            .filter(|p| !p.is_empty())
    }

    /// Sign this image as `version` for the device API: a version 2
    /// manifest naming `key` and the image's kind. The release key signs
    /// only a consumer build that builds it back in.
    pub fn offer(
        &self,
        key: &Key,
        version: u32,
        motion: Motion,
    ) -> Result<[u8; offer::MANIFEST_LEN]> {
        if key.scope() == KeyScope::Release {
            self.require_release_build(&key.public())?;
        }
        self.sign(key, version, motion)
    }

    /// Sign with the release key `key`, as [`Self::offer`] does, the
    /// consumer build that builds in `successor`, another release key,
    /// instead: how a feeder moves from one release key to the next. Its
    /// release slot takes one live key, and a key that signs key statements
    /// is a built-in key or the slot's, so no order of statements alone
    /// replaces the last one. Running this build, the successor is built
    /// in: it retires the old key and then fills the slot with itself, so
    /// bank 0, which keeps the key it was written with, trusts it too.
    ///
    /// The successor is a loaded key, never a typed public key, so the
    /// build can only hand feeders to a release key someone holds; load it
    /// with [`Key::load_release`].
    pub fn hand_over(
        &self,
        key: &Key,
        version: u32,
        motion: Motion,
        successor: &Key,
    ) -> Result<[u8; offer::MANIFEST_LEN]> {
        require(
            key.scope == KeyScope::Release && successor.scope == KeyScope::Release,
            || FrozenError::HandOverScope {
                signer: key.scope,
                successor: successor.scope,
            },
        )?;
        require(successor.public() != key.public(), || {
            FrozenError::HandOverToSelf
        })?;
        require(self.manifest.is_some(), || FrozenError::NoBuiltInKey)?;
        self.require_release_build(&successor.public())?;
        self.sign(key, version, motion)
    }

    fn sign(&self, key: &Key, version: u32, motion: Motion) -> Result<[u8; offer::MANIFEST_LEN]> {
        self.attended(motion)?;
        let kind = match self.kind {
            ImageKind::Build => OfferKind::Replacement,
            ImageKind::Stock => OfferKind::Stock,
        };
        let manifest = bank_manifest(&self.image, version, self.build_id)?;
        let signed = Signed {
            manifest,
            kind,
            model: self.model()?.code(),
            key_id: key.id(),
        };
        let body = offer::manifest_body(&signed);
        let mut out = [0u8; offer::MANIFEST_LEN];
        out[..body.len()].copy_from_slice(&body);
        out[body.len()..].copy_from_slice(&key.key.sign(&body).to_bytes());
        Ok(out)
    }

    /// The bank contents a manifest names: the image without its OTA
    /// container's header.
    pub fn payload(&self) -> &[u8] {
        self.image.get(CONTAINER_HEADER..).unwrap_or_default()
    }

    /// Refuse a build the release key should not sign. Each consumer
    /// feeder trusts the release key, and the image it installs then trusts
    /// only its own built-in key; so the release key signs only a consumer
    /// build with no variant (a test build) that builds in `built_in`,
    /// itself or its named successor. A bench image would hand the feeder
    /// to the bench key, whose seed lives in checkouts, and carries the
    /// bench hotspot's credentials. The bench key and an owner's key sign
    /// any build: a feeder takes those only where it built the bench key
    /// in or enrolled the key with a Dispense press. Stock has no
    /// provisioning, and a feeder takes it only with a downgrade statement
    /// made for that feeder.
    fn require_release_build(&self, built_in: &[u8; 32]) -> Result<()> {
        let Some(manifest) = &self.manifest else {
            return Ok(());
        };

        require(manifest.provisioning == Provisioning::Consumer, || {
            FrozenError::ReleaseProvisioning {
                provisioning: manifest.provisioning,
            }
        })?;
        if let Some(variant) = &manifest.variant {
            return Err(FrozenError::ReleaseVariant {
                variant: variant.clone(),
            }
            .into());
        }
        let builds_in = manifest
            .ota_public_key
            .as_deref()
            .and_then(|text| hex::decode_array_either_case::<32>(text.as_bytes()));
        require(builds_in == Some(*built_in), || {
            FrozenError::OtherBuiltIn {
                expected: hex::Lower(built_in).to_string(),
                built_in: manifest.ota_public_key.clone(),
            }
            .into()
        })
    }

    fn attended(&self, motion: Motion) -> Result<()> {
        if let Some(policy) = self.motion_policy() {
            require(motion == Motion::Attended(policy), || {
                FrozenError::Unattended {
                    policy: policy.to_owned(),
                }
            })?;
        }
        Ok(())
    }
}

/// The manifest fields of `container`'s bank contents, refusing what the
/// firmware would refuse.
fn bank_manifest(container: &[u8], version: u32, build_id: [u8; 32]) -> Result<Manifest> {
    require(container.len() > CONTAINER_HEADER + MARKER.len(), || {
        ImageError::TooShort
    })?;
    let payload = &container[CONTAINER_HEADER..];
    require(payload[..MARKER.len()] == MARKER, || ImageError::NoMarker)?;
    let image_len = u32::try_from(payload.len()).map_err(|_| ImageError::TooLarge)?;
    require(
        (wellfed_signed::SECTOR_BYTES..=wellfed_signed::BANK1_BYTES).contains(&image_len),
        || ImageError::Size { bytes: image_len },
    )?;

    Ok(Manifest {
        version,
        image_len,
        target_bank: 1,
        image_sha256: Sha256::digest(payload).into(),
        build_id,
    })
}

fn id_bytes(text: &str) -> Result<[u8; 32]> {
    hex::decode_array(text.as_bytes()).ok_or_else(|| FrozenError::BuildId.into())
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use wellfed_core::record::Tag;
    use wellfed_formats::bench::STOCK_BACKUPS;
    use wellfed_signed::SECTOR;

    use super::*;

    const BUILD: &str = "8a13e59efa15e56b5f5e32825909dc45e381a2027abc65d8de6658c40b302447";

    /// A frozen build whose manifest names `device`.
    fn build_for(device: &str) -> Frozen {
        Frozen {
            image: container(SECTOR, |i| u8::try_from(i % 251).unwrap()),
            kind: ImageKind::Build,
            build_id: [0x8a; 32],
            manifest: Some(BuildManifest {
                device: device.into(),
                ..BuildManifest::default()
            }),
        }
    }

    /// The model a manifest names is the build's own, never assumed: only a
    /// PLAF103 build signs, and a device no model has is refused.
    #[test]
    fn a_build_signs_as_the_model_its_manifest_names() {
        assert_eq!(build_for("plaf103").model().unwrap(), Model::Plaf103);
        assert!(matches!(
            build_for("plwf106").model(),
            Err(Error::Frozen(FrozenError::OtherModel {
                model: Model::Plwf106
            }))
        ));
        assert!(matches!(
            build_for("plaf999").model(),
            Err(Error::Frozen(FrozenError::UnknownDevice { device })) if device == "plaf999"
        ));
        let key = bench_key(3);
        assert!(
            build_for("plwf106")
                .offer(&key, 9, Motion::Refused)
                .is_err()
        );
    }

    /// An OTA container of `len` payload bytes starting with the marker.
    fn container(len: usize, fill: impl Fn(usize) -> u8) -> Vec<u8> {
        let mut image = vec![0u8; CONTAINER_HEADER];
        let mut payload: Vec<u8> = (0..len).map(fill).collect();
        payload[..8].copy_from_slice(&MARKER);
        image.extend(payload);
        image
    }

    /// A checkout with `private/`, and a frozen build review of `image`
    /// in it.
    fn freeze(image: &[u8], motion: Option<&str>) -> (tempfile::TempDir, std::path::PathBuf) {
        freeze_with(
            image,
            &json!({"startup_identity": BUILD, "startup_motion_policy": motion,
                    "firmware": "feeder"}),
        )
    }

    /// [`freeze`] with the build manifest's `fields`.
    fn freeze_with(
        image: &[u8],
        fields: &serde_json::Value,
    ) -> (tempfile::TempDir, std::path::PathBuf) {
        let checkout = tempfile::tempdir().unwrap();
        let review = checkout.path().join("private/review");
        wellfed_image::fixtures::freeze_build(checkout.path(), &review, image, fields).unwrap();
        (checkout, review)
    }

    fn build_image() -> Vec<u8> {
        wellfed_image::fixtures::image_carrying(
            format!("{} start build={BUILD} bank=", Tag::Start).as_bytes(),
        )
    }

    #[test]
    fn keygen_and_a_frozen_review_sign_what_the_firmware_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ota-key.json");
        let public = keygen(&path, KeyScope::Bench).unwrap();
        assert!(
            keygen(&path, KeyScope::Owner).is_err(),
            "keygen must not replace a key"
        );
        let file: OtaKeyFile = wellfed_formats::load(&path).unwrap();
        assert_eq!(file.scope, KeyScope::Bench);
        let key = Key::load(&path).unwrap();
        assert_eq!(hex::Lower(&key.public()).to_string(), public);

        let image = build_image();
        let (checkout, review) = freeze(&image, None);
        let frozen = Frozen::open(checkout.path(), &review).unwrap();
        assert_eq!(
            (frozen.kind(), frozen.build_id()),
            (ImageKind::Build, BUILD.into())
        );
        let signed = frozen.offer(&key, 9, Motion::Refused).unwrap();

        let parsed = offer::parse_manifest(&signed).unwrap();
        assert_eq!(
            (
                parsed.manifest.version,
                parsed.manifest.image_len as usize,
                hex::Lower(&parsed.manifest.build_id).to_string(),
                parsed.key_id
            ),
            (
                9,
                image.len() - CONTAINER_HEADER,
                BUILD.to_string(),
                key.id()
            )
        );

        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    /// The device API's manifest and statements, through the firmware's
    /// own checks.
    #[test]
    fn a_version_2_offer_and_its_statements_pass_the_firmware_s_checks() {
        use wellfed_signed::offer::{
            Context, KeyRing, KeyStatement, OfferError, Trust, admit_offer, admit_statement,
        };

        let key = bench_key(5);
        let (checkout, review) = freeze(&build_image(), None);
        let frozen = Frozen::open(checkout.path(), &review).unwrap();
        let signed = frozen.offer(&key, 9, Motion::Refused).unwrap();
        assert_eq!(
            frozen.payload().len(),
            build_image().len() - CONTAINER_HEADER
        );

        let built_in = [key.public()];
        let device = [0x42; 32];
        let context = |floor| Context {
            model: 1,
            ring: KeyRing {
                built_in: &built_in,
                added: None,
                enrolled: &[],
                revoked: &[],
            },
            floor,
            device,
            running: None,
        };
        let admitted = admit_offer(&signed, None, &context(8)).unwrap();
        assert_eq!(admitted.trust, Trust::BuiltIn);
        assert_eq!(admitted.signed.kind, OfferKind::Replacement);
        assert_eq!(
            hex::Lower(&admitted.signed.manifest.build_id).to_string(),
            BUILD
        );

        assert_eq!(
            admit_offer(&signed, None, &context(9)),
            Err(OfferError::Downgrade)
        );
        let statement = key.downgrade(&signed, &device, 9).unwrap();
        assert!(admit_offer(&signed, Some(&statement), &context(9)).is_ok());
        assert!(key.downgrade(&[0; 10], &device, 9).is_err());

        let ring = context(0).ring;
        assert_eq!(
            admit_statement(&key.revoke(&[7; 8]).unwrap(), &ring),
            Ok(KeyStatement::Revoke([7; 8]))
        );
        assert_eq!(
            admit_statement(&key.add_key(&[8; 32]).unwrap(), &ring),
            Ok(KeyStatement::Add([8; 32]))
        );
    }

    fn bench_key(seed: u8) -> Key {
        Key {
            key: SigningKey::from_bytes(&[seed; 32]),
            scope: KeyScope::Bench,
        }
    }

    /// A key file names its scope: one without it, or whose public key is
    /// not its seed's, is no key.
    #[test]
    fn a_key_file_names_its_scope_and_its_seed_s_public_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k.json");
        keygen(&path, KeyScope::Bench).unwrap();
        let mut file: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(file["scope"], json!("bench"));

        let mut unscoped = file.clone();
        unscoped.as_object_mut().unwrap().remove("scope");
        let edited = dir.path().join("edited.json");
        fs::write(&edited, unscoped.to_string()).unwrap();
        assert!(matches!(Key::load(&edited), Err(Error::Formats(_))));

        file["public_key"] = json!("00".repeat(32));
        fs::write(&edited, file.to_string()).unwrap();
        assert!(matches!(
            Key::load(&edited),
            Err(Error::Key(KeyError::PublicKey))
        ));
    }

    /// An owner's key is the same file under its own scope, and signs what
    /// a feeder that enrolled it admits; one that did not refuses it.
    #[test]
    fn an_owner_key_loads_and_signs_for_the_feeders_that_enrolled_it() {
        use wellfed_signed::offer::{Context, KeyError as RingError, KeyRing, OfferError, Trust};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("owner-key.json");
        let public = keygen(&path, KeyScope::Owner).unwrap();
        let file: OtaKeyFile = wellfed_formats::load(&path).unwrap();
        assert_eq!(file.scope, KeyScope::Owner);
        let key = Key::load(&path).unwrap();
        assert_eq!(hex::Lower(&key.public()).to_string(), public);

        let (checkout, review) = freeze(&build_image(), None);
        let signed = Frozen::open(checkout.path(), &review)
            .unwrap()
            .offer(&key, 9, Motion::Refused)
            .unwrap();
        const BENCH: [[u8; 32]; 1] = [[0x0b; 32]];
        fn context(enrolled: &[[u8; 32]]) -> Context<'_> {
            Context {
                model: 1,
                ring: KeyRing {
                    built_in: &BENCH,
                    added: None,
                    enrolled,
                    revoked: &[],
                },
                floor: 0,
                device: [0x42; 32],
                running: None,
            }
        }
        let admitted = offer::admit_offer(&signed, None, &context(&[key.public()])).unwrap();
        assert_eq!(admitted.trust, Trust::Enrolled);
        assert_eq!(
            offer::admit_offer(&signed, None, &context(&[])),
            Err(OfferError::Key(RingError::Unknown))
        );

        // The firmware takes an enrolled key's downgrade as it takes its
        // manifest, so the owner's key signs one.
        let enrolled = [key.public()];
        let at_floor = Context {
            floor: 9,
            ..context(&enrolled)
        };
        assert_eq!(
            offer::admit_offer(&signed, None, &at_floor),
            Err(OfferError::Downgrade)
        );
        let statement = key.downgrade(&signed, &[0x42; 32], 9).unwrap();
        let allowed = offer::admit_offer(&signed, Some(&statement), &at_floor).unwrap();
        assert_eq!(allowed.downgrade.map(|s| s.trust), Some(Trust::Enrolled));

        // Key statements are the built-in keys' alone: an owner's key
        // writes none.
        assert!(matches!(
            key.revoke(&[7; 8]),
            Err(Error::Key(KeyError::OwnerRevoke))
        ));
        assert!(matches!(
            key.add_key(&[8; 32]),
            Err(Error::Key(KeyError::OwnerAddKey))
        ));

        // A scope the format does not name is no key at all.
        let mut text: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        text["scope"] = json!("vendor");
        let other = dir.path().join("vendor.json");
        fs::write(&other, text.to_string()).unwrap();
        assert!(matches!(Key::load(&other), Err(Error::Formats(_))));
    }

    /// A stand-in made with `keygen` and moved offline becomes the release
    /// key by changing its one `scope` word, and then signs key statements.
    #[test]
    fn a_bench_key_relabelled_release_loads_as_the_release_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stand-in.json");
        let public = keygen(&path, KeyScope::Bench).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.matches(r#""scope": "bench""#).count(), 1, "{text}");
        fs::write(
            &path,
            text.replace(r#""scope": "bench""#, r#""scope": "release""#),
        )
        .unwrap();

        let key = Key::load(&path).unwrap();
        assert_eq!(key.scope(), KeyScope::Release);
        assert_eq!(hex::Lower(&key.public()).to_string(), public);
        assert!(key.revoke(&[7; 8]).is_ok());
        assert!(key.add_key(&key.public()).is_ok());
    }

    /// The release key fills the slot only with itself: any other key it
    /// added would land in the empty slot of every consumer feeder. The
    /// bench key, which only bench builds build in, still adds any key.
    #[test]
    fn the_release_key_adds_only_itself() {
        let release = Key {
            key: SigningKey::from_bytes(&[4; 32]),
            scope: KeyScope::Release,
        };
        let other = bench_key(5).public();
        assert!(matches!(
            release.add_key(&other),
            Err(Error::Key(KeyError::ReleaseAddsOther { added }))
                if added == hex::Lower(&offer::key_id(&other)).to_string()
        ));
        assert!(release.add_key(&release.public()).is_ok());
        assert!(bench_key(6).add_key(&other).is_ok());
    }

    /// No key signs its own retirement, whatever its scope: the statement
    /// could leave a feeder with no key to sign the next one.
    #[test]
    fn a_key_never_signs_its_own_retirement() {
        for scope in [KeyScope::Bench, KeyScope::Release] {
            let key = Key {
                key: SigningKey::from_bytes(&[4; 32]),
                scope,
            };
            assert!(matches!(
                key.revoke(&key.id()),
                Err(Error::Key(KeyError::SelfRevoke))
            ));
            assert!(key.revoke(&bench_key(5).id()).is_ok());
        }
    }

    /// `keygen` writes bench and owner keys only: the release key comes
    /// from `release_keygen`, which keeps it out of a checkout.
    #[test]
    fn keygen_refuses_the_release_scope() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("release.json");
        assert!(matches!(
            keygen(&path, KeyScope::Release),
            Err(Error::Key(KeyError::ReleaseKeygen))
        ));
        assert!(!path.exists());
    }

    #[test]
    fn an_installer_takes_only_a_manifest_the_built_in_key_signed() {
        let dir = tempfile::tempdir().unwrap();
        let (built_in, other) = (dir.path().join("built-in"), dir.path().join("other"));
        let public = keygen(&built_in, KeyScope::Bench).unwrap();
        keygen(&other, KeyScope::Bench).unwrap();
        let (checkout, review) = freeze(&build_image(), None);
        let frozen = Frozen::open(checkout.path(), &review).unwrap();
        let build = id_bytes(BUILD).unwrap();
        let signed = |path: &Path| {
            frozen
                .offer(&Key::load(path).unwrap(), 9, Motion::Refused)
                .unwrap()
        };

        require_built_in_signer(&public, &build, &signed(&built_in)).unwrap();

        assert!(matches!(
            require_built_in_signer(&public, &build, &signed(&other)),
            Err(Error::Frozen(FrozenError::NotBuiltInSigner { .. }))
        ));
        assert!(matches!(
            require_built_in_signer(&public, &[0; 32], &signed(&built_in)),
            Err(Error::Frozen(FrozenError::OtherBuild { .. }))
        ));
    }

    /// The installer's manifest passes the firmware's own admission with
    /// the built-in key: a damaged signature, or a correctly signed body
    /// naming another model, kind, bank or length, is refused.
    #[test]
    fn an_installer_manifest_must_pass_the_firmware_s_admission() {
        let key = bench_key(7);
        let fields = json!({"startup_identity": BUILD, "firmware": "feeder",
                            "ota_public_key": hex::Lower(&key.public()).to_string()});
        let (checkout, review) = freeze_with(&build_image(), &fields);
        let frozen = Frozen::open(checkout.path(), &review).unwrap();
        let signed = frozen.offer(&key, 9, Motion::Refused).unwrap();
        frozen.require_installable(&signed).unwrap();

        let body = offer::MANIFEST_BODY;
        for at in body..offer::MANIFEST_LEN {
            let mut damaged = signed;
            damaged[at] ^= 0x01;
            assert!(
                matches!(
                    frozen.require_installable(&damaged),
                    Err(Error::Frozen(FrozenError::NotAdmitted {
                        error: offer::OfferError::Signature
                    }))
                ),
                "signature byte {at}"
            );
        }

        let resigned = |edit: &dyn Fn(&mut Signed)| {
            let mut parsed = offer::parse_manifest(&signed).unwrap();
            edit(&mut parsed);
            let body = offer::manifest_body(&parsed);
            let mut out = body.to_vec();
            out.extend_from_slice(&key.key.sign(&body).to_bytes());
            frozen.require_installable(&out).unwrap_err()
        };
        assert!(matches!(
            resigned(&|s| s.model = Model::Plwf106.code()),
            Error::Frozen(FrozenError::NotAdmitted {
                error: offer::OfferError::Model
            })
        ));
        assert!(matches!(
            resigned(&|s| s.manifest.target_bank = 0),
            Error::Frozen(FrozenError::NotAdmitted {
                error: offer::OfferError::Bank
            })
        ));
        assert!(matches!(
            resigned(&|s| s.kind = OfferKind::Stock),
            Error::Frozen(FrozenError::NotAdmitted { .. } | FrozenError::OtherKind)
        ));
        assert!(matches!(
            resigned(&|s| s.manifest.image_len += 1),
            Error::Frozen(FrozenError::OtherLength { .. })
        ));
    }

    #[test]
    fn a_manifest_for_other_bytes_of_the_same_build_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key");
        keygen(&path, KeyScope::Bench).unwrap();
        let key = Key::load(&path).unwrap();
        let (checkout, review) = freeze(&build_image(), None);
        let frozen = Frozen::open(checkout.path(), &review).unwrap();
        let other_bytes = wellfed_image::fixtures::image_carrying(
            format!("{} start build={BUILD} bank=1", Tag::Start).as_bytes(),
        );
        let (other_checkout, other_review) = freeze(&other_bytes, None);
        let other = Frozen::open(other_checkout.path(), &other_review).unwrap();
        let manifest = frozen.offer(&key, 9, Motion::Refused).unwrap();

        frozen.require_same_image(&manifest).unwrap();

        assert_eq!(other.build_id(), frozen.build_id());
        assert!(matches!(
            other.require_same_image(&manifest),
            Err(Error::Frozen(FrozenError::OtherImage { .. }))
        ));
    }

    #[test]
    fn signing_refuses_images_the_firmware_would_refuse() {
        let refused = |image: &[u8]| bank_manifest(image, 1, [0; 32]).err();

        let mut no_marker = container(SECTOR, |_| 0);
        no_marker[CONTAINER_HEADER] = b'0';
        assert!(matches!(
            refused(&no_marker),
            Some(Error::Image(ImageError::NoMarker))
        ));
        assert!(matches!(
            refused(&container(100, |_| 0)),
            Some(Error::Image(ImageError::Size { bytes: 100 }))
        ));
        let large = container(wellfed_signed::BANK1_BYTES as usize + 1, |_| 0);
        assert!(matches!(
            refused(&large),
            Some(Error::Image(ImageError::Size { .. }))
        ));
        assert!(refused(&container(SECTOR, |_| 0)).is_none());
    }

    fn refusal(checkout: &Path, review: &Path) -> Error {
        Frozen::open(checkout, review).unwrap_err()
    }

    #[test]
    fn an_image_that_differs_from_its_review_is_refused() {
        use wellfed_image::review::ReviewError as Review;

        let image = build_image();
        let (checkout, review) = freeze(&image, None);

        // No review.json: nothing says what was reviewed.
        let described = fs::read(review.join("review.json")).unwrap();
        fs::remove_file(review.join("review.json")).unwrap();
        let error = refusal(checkout.path(), &review);
        assert!(
            matches!(&error, Error::Frozen(FrozenError::NoReview { review: path }) if *path == review),
            "{error:?}"
        );
        fs::write(review.join("review.json"), &described).unwrap();

        // A swapped image: a valid container with the same identity.
        let other = wellfed_image::fixtures::image_carrying(
            format!("PLFW1 start build={BUILD} bank=1").as_bytes(),
        );
        fs::write(review.join("image.bin"), &other).unwrap();
        let error = refusal(checkout.path(), &review);
        assert!(
            matches!(
                error,
                Error::Review(wellfed_image::Error::Review(Review::ImageMismatch))
            ),
            "{error:?}"
        );
        fs::write(review.join("image.bin"), &image).unwrap();

        // An edited build ID or motion policy in the manifest.
        let manifest = fs::read_to_string(review.join("build-manifest.json")).unwrap();
        for edited in [
            manifest.replace(BUILD, &"ab".repeat(32)),
            manifest.replace("null", "\"fw16-pulse-v1\""),
        ] {
            fs::write(review.join("build-manifest.json"), edited).unwrap();
            let error = refusal(checkout.path(), &review);
            assert!(
                matches!(
                    error,
                    Error::Review(wellfed_image::Error::Review(Review::ManifestMismatch))
                ),
                "{error:?}"
            );
        }
    }

    #[test]
    fn a_review_of_an_image_without_its_identity_is_refused() {
        let image = wellfed_image::fixtures::image_carrying(b"no identity here");
        let (checkout, review) = freeze(&image, None);
        let error = refusal(checkout.path(), &review);
        assert!(
            matches!(&error, Error::Frozen(FrozenError::NoIdentity { identity }) if identity == BUILD),
            "{error:?}"
        );
    }

    #[test]
    fn a_motion_image_needs_its_exact_policy_attended() {
        let key = bench_key(2);
        let (checkout, review) = freeze(&build_image(), Some("fw16-pulse-v1"));
        let frozen = Frozen::open(checkout.path(), &review).unwrap();
        assert_eq!(frozen.motion_policy(), Some("fw16-pulse-v1"));

        for motion in [Motion::Refused, Motion::Attended("fw16-other-v1")] {
            let error = frozen.offer(&key, 1, motion).unwrap_err();
            assert!(
                matches!(&error, Error::Frozen(FrozenError::Unattended { policy }) if policy == "fw16-pulse-v1"),
                "{error:?}"
            );
        }
        assert!(
            frozen
                .offer(&key, 1, Motion::Attended("fw16-pulse-v1"))
                .is_ok()
        );
    }

    /// The release key signs only a consumer build with no variant that
    /// keeps the release key built in; the bench key and an owner's key,
    /// which a consumer feeder takes only once it enrolled them, sign any.
    #[test]
    fn a_key_signs_only_the_builds_its_scope_provisions() {
        let scoped = |seed: u8, scope| Key {
            key: SigningKey::from_bytes(&[seed; 32]),
            scope,
        };
        let release = scoped(1, KeyScope::Release);
        let bench = scoped(2, KeyScope::Bench);
        let owner = scoped(3, KeyScope::Owner);
        let release_public = hex::Lower(&release.public()).to_string();
        let bench_public = hex::Lower(&bench.public()).to_string();

        let review = |provisioning: &str, variant: Option<&str>, built_in: &str| {
            let fields = json!({"startup_identity": BUILD, "firmware": "feeder",
                                "provisioning": provisioning, "variant": variant,
                                "ota_public_key": built_in});
            let (checkout, review) = freeze_with(&build_image(), &fields);
            let frozen = Frozen::open(checkout.path(), &review).unwrap();
            (checkout, frozen)
        };
        let (_consumer_dir, consumer) = review("consumer", None, &release_public);
        let (_upper_dir, upper) = review("consumer", None, &release_public.to_uppercase());
        let (_bench_dir, bench_build) = review("bench", None, &bench_public);
        let (_variant_dir, variant) = review("consumer", Some("virtual-panel"), &release_public);
        let (_other_dir, other_key) = review("consumer", None, &bench_public);

        for (frozen, key) in [
            (&consumer, &release),
            (&upper, &release),
            (&bench_build, &bench),
            (&consumer, &bench),
            (&variant, &bench),
            (&consumer, &owner),
            (&bench_build, &owner),
        ] {
            frozen.offer(key, 9, Motion::Refused).unwrap();
        }

        let refused =
            |frozen: &Frozen, key: &Key| frozen.offer(key, 9, Motion::Refused).unwrap_err();
        assert!(matches!(
            refused(&bench_build, &release),
            Error::Frozen(FrozenError::ReleaseProvisioning {
                provisioning: Provisioning::Bench
            })
        ));
        assert!(matches!(
            refused(&variant, &release),
            Error::Frozen(FrozenError::ReleaseVariant { variant }) if variant == "virtual-panel"
        ));
        assert!(matches!(
            refused(&other_key, &release),
            Error::Frozen(FrozenError::OtherBuiltIn { built_in: Some(key), .. })
                if key == bench_public
        ));

        // A consumer build that names no key at all is refused too.
        let fields = json!({"startup_identity": BUILD, "firmware": "feeder",
                            "provisioning": "consumer"});
        let (checkout, keyless) = freeze_with(&build_image(), &fields);
        let keyless = Frozen::open(checkout.path(), &keyless).unwrap();
        assert!(matches!(
            refused(&keyless, &release),
            Error::Frozen(FrozenError::OtherBuiltIn { built_in: None, .. })
        ));
    }

    /// The release key hands a consumer build over only to the other
    /// release key that build builds in; a build that builds the signer
    /// itself in needs no hand-over.
    #[test]
    fn the_release_key_hands_over_only_to_the_successor_the_build_builds_in() {
        let scoped = |seed: u8, scope| Key {
            key: SigningKey::from_bytes(&[seed; 32]),
            scope,
        };
        let release = scoped(1, KeyScope::Release);
        let successor = scoped(6, KeyScope::Release);
        let builds_in = |key: &Key, provisioning: &str, variant: Option<&str>| {
            let fields = json!({"startup_identity": BUILD, "firmware": "feeder",
                                "provisioning": provisioning, "variant": variant,
                                "ota_public_key": hex::Lower(&key.public()).to_string()});
            let (checkout, review) = freeze_with(&build_image(), &fields);
            let frozen = Frozen::open(checkout.path(), &review).unwrap();
            (checkout, frozen)
        };
        let (_dir, frozen) = builds_in(&successor, "consumer", None);
        let hand_over =
            |frozen: &Frozen, key: &Key, to: &Key| frozen.hand_over(key, 9, Motion::Refused, to);

        assert!(matches!(
            frozen.offer(&release, 9, Motion::Refused),
            Err(Error::Frozen(FrozenError::OtherBuiltIn { .. }))
        ));
        let signed = hand_over(&frozen, &release, &successor).unwrap();
        assert_eq!(offer::parse_manifest(&signed).unwrap().key_id, release.id());

        // Another release key than the one the build builds in.
        assert!(matches!(
            hand_over(&frozen, &release, &scoped(7, KeyScope::Release)),
            Err(Error::Frozen(FrozenError::OtherBuiltIn { .. }))
        ));
        // Both keys are release keys: a bench stand-in or an owner's key is
        // no successor, and only the release key hands over.
        for (signer, to) in [
            (&release, &scoped(6, KeyScope::Bench)),
            (&release, &scoped(6, KeyScope::Owner)),
            (&scoped(1, KeyScope::Bench), &successor),
        ] {
            assert!(matches!(
                hand_over(&frozen, signer, to),
                Err(Error::Frozen(FrozenError::HandOverScope { .. }))
            ));
        }
        let (_dir, own) = builds_in(&release, "consumer", None);
        assert!(matches!(
            hand_over(&own, &release, &release),
            Err(Error::Frozen(FrozenError::HandOverToSelf))
        ));

        // The hand-over is still a release build: no bench build, no variant.
        for (provisioning, variant) in [("bench", None), ("consumer", Some("virtual-panel"))] {
            let (_dir, frozen) = builds_in(&successor, provisioning, variant);
            assert!(hand_over(&frozen, &release, &successor).is_err());
        }
    }

    /// A successor loads only as a release key from outside any checkout:
    /// the bench stand-in `keygen` writes into `private/` is neither.
    #[test]
    fn a_successor_loads_only_as_a_release_key_off_any_checkout() {
        let media = tempfile::tempdir().unwrap();
        let release = media.path().join("successor.json");
        let release = release_keygen(&release).unwrap().private;
        Key::load_release(&release).unwrap();

        let stand_in = media.path().join("stand-in.json");
        keygen(&stand_in, KeyScope::Bench).unwrap();
        assert!(matches!(
            Key::load_release(&stand_in),
            Err(Error::Key(KeyError::NotRelease {
                scope: KeyScope::Bench,
                ..
            }))
        ));

        let checkout = tempfile::tempdir().unwrap();
        fs::create_dir_all(checkout.path().join(".git")).unwrap();
        fs::create_dir_all(checkout.path().join("private")).unwrap();
        let inside = checkout.path().join("private/release-standin-key.json");
        fs::copy(&release, &inside).unwrap();
        assert!(matches!(
            Key::load_release(&inside),
            Err(Error::Key(KeyError::InCheckout { .. }))
        ));
    }

    /// An owner's backup signs as stock only when it is the pinned stock
    /// container: a build handed in as a backup is refused before anything
    /// is signed.
    #[test]
    fn only_the_pinned_stock_image_signs_without_a_review() {
        let error = Frozen::stock(build_image()).unwrap_err();
        assert!(
            matches!(error, Error::Frozen(FrozenError::NotStock)),
            "{error:?}"
        );
    }

    /// A review that calls an image stock signs only the pinned stock
    /// image: otherwise any build, a motion build included, could be
    /// relabelled to skip the motion and health checks.
    #[test]
    fn a_stock_review_of_another_image_is_refused() {
        let image = build_image();
        let (checkout, review) = freeze(&image, Some("fw16-pulse-v1"));
        fs::remove_file(review.join("build-manifest.json")).unwrap();
        let mut frozen: serde_json::Value =
            serde_json::from_slice(&fs::read(review.join("review.json")).unwrap()).unwrap();
        frozen["image_kind"] = json!("stock");
        frozen["manifest_sha256"] = serde_json::Value::Null;
        frozen["review"] = json!({"stock_sha256": frozen["ota_sha256"]});
        fs::write(review.join("review.json"), frozen.to_string()).unwrap();

        let error = refusal(checkout.path(), &review);
        assert!(
            matches!(
                error,
                Error::Review(wellfed_image::Error::Review(
                    wellfed_image::review::ReviewError::NotOriginalStock
                ))
            ),
            "{error:?}"
        );
    }

    /// The next version is picked above every manifest kept in a
    /// directory; anything that is not a manifest is passed over.
    #[test]
    fn the_highest_signed_version_is_read_from_the_headers() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(highest_signed(dir.path()), None);
        assert_eq!(highest_signed(&dir.path().join("missing")), None);

        let manifest = |version: u32| {
            let mut bytes = vec![0u8; offer::MANIFEST_LEN];
            bytes[..8].copy_from_slice(&offer::MAGIC);
            bytes[8..12].copy_from_slice(&version.to_le_bytes());
            bytes
        };
        fs::write(dir.path().join("a-v3.manifest"), manifest(3)).unwrap();
        fs::write(dir.path().join("b-v12.manifest"), manifest(12)).unwrap();
        fs::write(dir.path().join("c.bin"), manifest(40)).unwrap();
        fs::write(dir.path().join("d.manifest"), b"not a manifest").unwrap();

        let (version, path) = highest_signed(dir.path()).unwrap();
        assert_eq!(version, 12);
        assert!(path.ends_with("b-v12.manifest"));
    }

    /// The bench's manifests and the installs' kept promotions are one
    /// sequence: the highest of either is what both sign above.
    #[test]
    fn versions_signed_so_far_cover_the_bench_and_the_installs() {
        let root = tempfile::tempdir().unwrap();
        let backups = Path::new(STOCK_BACKUPS);
        assert_eq!(signed_so_far(root.path(), backups), None);

        let manifest = |version: u32| {
            let mut bytes = vec![0u8; offer::MANIFEST_LEN];
            bytes[..8].copy_from_slice(&offer::MAGIC);
            bytes[8..12].copy_from_slice(&version.to_le_bytes());
            bytes
        };
        let verification = root.path().join(VERIFICATION);
        let feeder = root.path().join(STOCK_BACKUPS).join("ab".repeat(32));
        fs::create_dir_all(&verification).unwrap();
        fs::create_dir_all(&feeder).unwrap();
        fs::write(verification.join("unit-v7.manifest"), manifest(7)).unwrap();
        assert_eq!(signed_so_far(root.path(), backups).unwrap().0, 7);

        fs::write(feeder.join("promotion-v9.manifest"), manifest(9)).unwrap();
        let (version, path) = signed_so_far(root.path(), backups).unwrap();
        assert_eq!(version, 9);
        assert!(path.ends_with("promotion-v9.manifest"));

        // An absolute directory elsewhere counts as given.
        let elsewhere = tempfile::tempdir().unwrap();
        let other = elsewhere.path().join("cd".repeat(32));
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("promotion-v11.manifest"), manifest(11)).unwrap();
        assert_eq!(signed_so_far(root.path(), elsewhere.path()).unwrap().0, 11);
    }
}
