//! Freeze a reviewed development image offline: `wellfed-build review`.
//!
//! A frozen review is the only form in which `wellfed-flash` accepts an
//! image to install. For a first-boot build of either model it re-derives the
//! build's inputs from the current sources, re-hashes every input, overlay,
//! artifact and archive the manifest names, and re-runs the linked-code
//! audits. It then executes the actual factory loader (`wellfed-emu`) on
//! the model's pinned flash (the PLAF103's baseline, the PLWF106's backup)
//! with the image alone in each bank: both models run the same loader. The
//! PLAF103's original stock image is frozen by hash only; the fountain has
//! no stock OTA. No serial port opens, and nothing claims the application
//! boots.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use wellfed_core::record::Tag;
use wellfed_emu::BANKS;
use wellfed_emu::factory::{self, Entry};
use wellfed_formats::canonical::{canonical_of, sha256};
use wellfed_formats::manifest::BuildManifest;
use wellfed_formats::private::{sync_directory, write_new};
use wellfed_formats::require;
use wellfed_formats::review::{
    BuildDetails, Details, FactoryRun, ImageKind, ImageReview, MappingCase, ReviewKind,
    SCHEMA_VERSION, Stop,
};
use wellfed_image::bank::{BANK1, PLWF106_BANK1};
use wellfed_image::ota::{STOCK_SHA256, container, images};
use wellfed_image::root::Root;
use wellfed_proto::device::Model;

use crate::identity::current_inputs;
use crate::image::{audit_build, hold_output};
use crate::platform::{Linked, amebad};
use crate::{Device, Firmware, Profile, Repo, Result, digest, revision};

/// Why an image could not be frozen as a review. Each is terminal: nothing
/// is written until every check passed.
#[derive(Debug, thiserror::Error)]
pub enum ReviewError {
    #[error("New private output required")]
    Output,
    #[error("Pinned stock image required")]
    Stock,
    #[error("Private build required")]
    PrivateBuild,
    #[error("Only a reviewed UART startup profile is supported, not {device} {profile}")]
    Profile { device: String, profile: String },
    #[error("The fountain's flash fixture is not its pinned backup")]
    Backup,
    #[error("Build manifest names no firmware; rebuild it")]
    NoFirmware,
    #[error("Unexpected SDK/toolchain")]
    SdkToolchain,
    #[error("SDK worktree revision differs")]
    SdkRevision,
    /// The manifest's inputs are not the ones a build of the current
    /// sources would have; `name` is the first that differs or is missing
    /// on either side.
    #[error("Build input {name} is stale or missing; rebuild before review")]
    StaleInputs { name: String },
    #[error("Build input {name} differs")]
    Input { name: String },
    #[error("SDK overlay {name} differs")]
    Overlay { name: String },
    /// The manifest's startup identity is not the one its own device,
    /// sources, firmware, variant, provisioning and key give.
    #[error("Build manifest's startup identity {recorded:?} is not its sources' {derived}")]
    Identity {
        recorded: Option<String>,
        derived: String,
    },
    /// The image does not print the identity its manifest records, so the
    /// manifest does not describe it.
    #[error("The image does not carry its manifest's startup identity {identity}")]
    NoIdentity { identity: String },
    #[error("Both core packages must exactly compose the OTA payload")]
    Composition,
    #[error("Duplicate core archive {core}")]
    DuplicateCore { core: String },
    #[error("Compiled archive {path} differs")]
    Archive { path: String },
    #[error("Both compiled cores required")]
    Cores,
    /// A rerun audit differs from the one the build recorded, by the
    /// manifest's name for it.
    #[error("The rerun {audit} differs from the build manifest's")]
    Audits { audit: &'static str },
    /// The audits and emulation take long enough for an edit to land.
    #[error("Build changed during review")]
    Changed,
    #[error("Artifact {path} changed during review")]
    ArtifactChanged { path: String },
    #[error("Build artifact {path} differs")]
    Artifact { path: String },
    #[error("Incomplete packaged artifacts")]
    Artifacts,
    #[error("Manifest file {name} escapes its root")]
    Escapes { name: String },
    #[error("Missing km4_xip image")]
    NoKm4Xip,
    /// The flash image a loader case starts from is too short to hold the
    /// image in the bank the case places it in.
    #[error("The flash image is {bytes} bytes, short of the {needed} the bank case needs")]
    ShortFlash { bytes: usize, needed: usize },
    #[error("Bank {bank}: the actual factory instructions reject the diagnostic headers")]
    FactoryCheck { bank: u8 },
    #[error("Bank {bank}: unexpected factory mapper sequence")]
    MapperSequence { bank: u8 },
    #[error("Bank {bank}: the factory mapping does not cover packaged core {core}")]
    Mapping { bank: u8, core: usize },
    #[error("Bank {bank}: the single-valid diagnostic bank selection differs")]
    Selection { bank: u8 },
    /// With the pinned flash's bank 0 left valid and the image in bank 1, as
    /// an install leaves them, the loader did not choose bank 1.
    #[error("The loader does not choose the image installed in bank 1")]
    Installed { stop: Stop, result: u32 },
    /// Beside the installed image, bank 0 no longer passes the loader's own
    /// check, so the case would not show bank 1 winning over valid stock.
    #[error("Bank 0 fails the loader's check beside the installed image")]
    InstalledStock { stop: Stop, result: u32 },
}

const ARTIFACTS: [&str; 4] = [
    "OTA_All.bin",
    "km0_image2_all.bin",
    "km0_km4_image2.bin",
    "km4_image2_all.bin",
];
/// The device whose stock OTA a stock review freezes.
const DEVICE: &str = Model::Plaf103.name();
const SCOPE: &str =
    "Package, linked code, stock pulls and modeled mapping; application behavior requires review";

/// Review `kind` (for a build, the one in `build`) and freeze it as a new
/// private directory `output`: `image.bin`, `review.json` and, for a build,
/// `build-manifest.json`.
pub fn freeze(repo: &Repo, kind: ImageKind, build: &Path, output: &Path) -> Result<ImageReview> {
    let root = Root::new(&repo.root)?;
    let (output, build) = (repo.root.join(output), repo.root.join(build));
    require(!output.exists() && root.is_private(&output), || {
        ReviewError::Output
    })?;

    let (image, manifest, review, model) = match kind {
        ImageKind::Stock => {
            let image = fs::read(Device::new(repo, DEVICE)?.stock(repo)?.image)?;
            require(sha256(&image) == STOCK_SHA256, || ReviewError::Stock)?;
            let details = Details::Stock {
                stock_sha256: STOCK_SHA256.into(),
            };
            (image, None, details, Model::Plaf103)
        }
        ImageKind::Build => {
            require(root.is_private(&build), || ReviewError::PrivateBuild)?;
            let (image, manifest, details, model) = review(repo, &build)?;
            (
                image,
                Some(manifest),
                Details::Build(Box::new(details)),
                model,
            )
        }
    };

    let payload = model_payload(&image, model)?;
    let result = ImageReview {
        schema_version: SCHEMA_VERSION,
        kind: match model {
            Model::Plaf103 => ReviewKind::Plaf103DevelopmentImage,
            Model::Plwf106 => ReviewKind::Plwf106DevelopmentImage,
        },
        image_kind: kind,
        ota_sha256: sha256(&image),
        payload_sha256: sha256(payload),
        payload_bytes: payload.len() as u64,
        manifest_sha256: manifest.as_deref().map(sha256),
        review,
        reviewer_sha256: digest(&std::env::current_exe()?)?,
        application_boot_verified: false,
    };

    root.output_directory(&output)?;
    let mut review_json = canonical_of(&result)?;
    review_json.push(b'\n');
    write_new(&output.join("image.bin"), &image)?;
    write_new(&output.join("review.json"), &review_json)?;
    if let Some(manifest) = &manifest {
        write_new(&output.join("build-manifest.json"), manifest)?;
    }
    sync_directory(&output)?;
    if let Some(parent) = output.parent() {
        sync_directory(parent)?;
    }
    Ok(result)
}

/// The application payload, sector aligned within the PLAF103's bank
/// footprint, with both cores at their exact addresses.
pub fn bank_payload(image: &[u8]) -> Result<&[u8]> {
    model_payload(image, Model::Plaf103)
}

/// [`bank_payload`] within `model`'s bank-1 window, by the check the flash
/// client makes of the same image.
fn model_payload(image: &[u8], model: Model) -> Result<&[u8]> {
    let bank = match model {
        Model::Plaf103 => &BANK1,
        Model::Plwf106 => &PLWF106_BANK1,
    };
    Ok(wellfed_image::bank::payload(image, bank)?)
}

/// Everything a build review checks, returning the image, the exact manifest
/// bytes, the review details and the model the build is for.
fn review(repo: &Repo, build: &Path) -> Result<(Vec<u8>, Vec<u8>, BuildDetails, Model)> {
    let _held = hold_output(build)?;
    let raw = fs::read(build.join("manifest.json"))?;
    let m: BuildManifest = serde_json::from_slice(&raw)?;
    let lock = amebad::lock(repo)?;
    let model = Model::ALL
        .into_iter()
        .find(|model| model.name() == m.device);
    let Some(model) = model.filter(|_| m.profile == Profile::Firstboot.name()) else {
        return Err(ReviewError::Profile {
            device: m.device.clone(),
            profile: m.profile.clone(),
        }
        .into());
    };
    let device = Device::new(repo, &m.device)?;
    let Some(firmware) = m.firmware.as_deref() else {
        return Err(ReviewError::NoFirmware.into());
    };
    let firmware = Firmware::new(repo, firmware)?;
    require(
        m.sdk_revision == lock.sdk.revision && m.toolchain_sha256 == lock.toolchain.sha256,
        || ReviewError::SdkToolchain,
    )?;
    let sdk = build.join("sdk");
    require(revision(&sdk)? == m.sdk_revision, || {
        ReviewError::SdkRevision
    })?;

    // Inputs, overlays, artifacts and archives, each re-hashed.
    let inputs = current_inputs(repo, &device, Profile::Firstboot, m.provisioning, &firmware)?;
    if let Some(name) = first_difference(&m.inputs, &inputs) {
        return Err(ReviewError::StaleInputs { name }.into());
    }
    for (name, expected) in &m.inputs {
        require(digest(&local(&repo.root, name)?)? == *expected, || {
            ReviewError::Input { name: name.clone() }
        })?;
    }
    for (name, expected) in &m.sdk_overrides {
        require(digest(&local(&sdk, name)?)? == *expected, || {
            ReviewError::Overlay { name: name.clone() }
        })?;
    }
    let files = artifacts(build, &m)?;
    let file = |name: &str| files.get(name).map_or(&[][..], Vec::as_slice);
    let image = file("OTA_All.bin").to_vec();
    let payload = model_payload(&image, model)?;
    identity(&m, payload)?;
    require(
        payload == file("km0_km4_image2.bin")
            && payload == [file("km0_image2_all.bin"), file("km4_image2_all.bin")].concat(),
        || ReviewError::Composition,
    )?;
    let mut cores: Vec<&str> = Vec::new();
    for archive in &m.compiled_archives {
        require(!cores.contains(&archive.core.as_str()), || {
            ReviewError::DuplicateCore {
                core: archive.core.clone(),
            }
        })?;
        let path = local(build, &archive.path)?;
        require(digest(&path)? == archive.sha256, || ReviewError::Archive {
            path: archive.path.clone(),
        })?;
        cores.push(&archive.core);
    }
    cores.sort_unstable();
    require(cores == ["KM0", "KM4"], || ReviewError::Cores)?;

    // The linked-code audits, rerun, must be the ones the build recorded:
    // every one, so a record nothing reconciled cannot ride in a review.
    let Linked::Amebad(fresh) = audit_build(repo, build)?;
    if let Some(audit) = fresh.differs_from(&m.audits) {
        return Err(ReviewError::Audits { audit }.into());
    }

    let (baseline, rom) = match model {
        Model::Plaf103 => factory::inputs(&repo.root)?,
        Model::Plwf106 => {
            let backup = fs::read(device.stock(repo)?.image)?;
            require(
                sha256(&backup) == amebad::audit::pinmap::reference_sha256(model),
                || ReviewError::Backup,
            )?;
            (backup, factory::rom(&repo.root)?)
        }
    };
    let factory_mapping = mapping_cases(&baseline, &rom, &image)?;
    installed_selection(&baseline, &rom, &image, model)?;

    // Refuse edits made during the (long) audits and emulation.
    require(
        fs::read(build.join("manifest.json"))? == raw
            && current_inputs(repo, &device, Profile::Firstboot, m.provisioning, &firmware)?
                == inputs,
        || ReviewError::Changed,
    )?;
    for artifact in &m.artifacts {
        require(
            digest(&local(build, &artifact.path)?)? == artifact.sha256,
            || ReviewError::ArtifactChanged {
                path: artifact.path.clone(),
            },
        )?;
    }

    let details = BuildDetails {
        startup_mains: fresh.firstboot_startup_audit.mains,
        layout: fresh.layout_audit,
        pinmap: fresh.km0_pinmap_audit,
        factory_mapping,
        scope: SCOPE.into(),
    };
    Ok((image, raw, details, model))
}

/// The first key, in order, that `recorded` and `current` do not both hold
/// with one digest.
fn first_difference(
    recorded: &BTreeMap<String, String>,
    current: &BTreeMap<String, String>,
) -> Option<String> {
    recorded
        .keys()
        .chain(current.keys())
        .filter(|name| recorded.get(*name) != current.get(*name))
        .min()
        .cloned()
}

/// Refuse a manifest whose startup identity is not the one its own fields
/// give, or that the image's payload does not print. Together they bind the
/// variant, the provisioning and the update key the manifest declares to the
/// image being frozen.
fn identity(m: &BuildManifest, payload: &[u8]) -> Result<()> {
    let derived = m.source_identity()?;
    require(
        m.startup_identity.as_deref() == Some(derived.as_str()),
        || ReviewError::Identity {
            recorded: m.startup_identity.clone(),
            derived: derived.clone(),
        },
    )?;
    let marker = format!("{} start build={derived} ", Tag::Start);
    require(
        payload
            .windows(marker.len())
            .any(|window| window == marker.as_bytes()),
        || ReviewError::NoIdentity {
            identity: derived.clone(),
        },
    )?;
    Ok(())
}

/// The four packaged artifacts by file name, each matching its manifest entry.
fn artifacts(build: &Path, m: &BuildManifest) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut files = BTreeMap::new();
    for artifact in &m.artifacts {
        let path = local(build, &artifact.path)?;
        let data = fs::read(&path)?;
        require(
            data.len() as u64 == artifact.bytes && sha256(&data) == artifact.sha256,
            || ReviewError::Artifact {
                path: artifact.path.clone(),
            },
        )?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        files.insert(name.unwrap_or_default(), data);
    }
    require(
        files.len() == m.artifacts.len() && files.keys().eq(ARTIFACTS),
        || ReviewError::Artifacts,
    )?;
    Ok(files)
}

/// A regular file `name` inside `parent`.
fn local(parent: &Path, name: &str) -> Result<PathBuf> {
    let path = parent.join(name).canonicalize()?;
    require(
        path.starts_with(parent.canonicalize()?) && path.is_file(),
        || ReviewError::Escapes { name: name.into() },
    )?;
    Ok(path)
}

/// The actual factory loader on `current` with `image` as the only valid
/// bank, for each bank: its bank check maps exactly the packaged cores, and
/// the ordinary selection boots it. Nothing here is ever sent to a device.
pub fn mapping_cases(current: &[u8], rom: &[u8], image: &[u8]) -> Result<Vec<MappingCase>> {
    let payload = container(image)?;
    let split = images(payload)?
        .iter()
        .find(|region| region.name == "km4_xip")
        .map(|region| region.payload_header_offset)
        .ok_or(ReviewError::NoKm4Xip)?;

    let mut cases = Vec::new();
    for (bank, start) in (0u8..).zip(BANKS) {
        let start = start as usize;
        let mut fixture = placed(current, start, payload)?;
        // This fixture deliberately selects just this bank.
        let other = BANKS[usize::from(1 - bank)] as usize;
        let Some(header) = fixture.get_mut(other..other + 8) else {
            return Err(ReviewError::ShortFlash {
                bytes: current.len(),
                needed: other + 8,
            }
            .into());
        };
        header.fill(0);

        let checked = factory::execute(&fixture, rom, Entry::Check(bank))?;
        require(
            checked.stop == Stop::Returned && checked.result == 1,
            || ReviewError::FactoryCheck { bank },
        )?;
        require(
            checked.configs.len() == 4 && checked.commands == [[0, 1], [1, 1]],
            || ReviewError::MapperSequence { bank },
        )?;
        let cores = [
            (0x0C00_0000u32, start, split),
            (0x0E00_0000, start + split, payload.len() - split),
        ];
        for (index, (virtual_start, physical, length)) in cores.into_iter().enumerate() {
            require(
                maps(&checked, index, virtual_start, physical, length),
                || ReviewError::Mapping { bank, core: index },
            )?;
        }

        let chosen = factory::execute(&fixture, rom, Entry::Select)?;
        require(
            chosen.stop == Stop::Selected && chosen.result == u32::from(bank) + 1,
            || ReviewError::Selection { bank },
        )?;
        cases.push(MappingCase {
            bank,
            bank_check: checked,
            selection: chosen,
        });
    }
    Ok(cases)
}

/// The installed configuration: `current` as pinned, with the image in bank
/// 1. The loader tries bank 1 first, so it must choose the image there.
///
/// On the PLWF106 the pinned backup's bank 0 is stock, which every fountain
/// install keeps exactly (a give-way back to it requires that), so bank 0
/// must also pass the loader's own check: bank 1 wins over valid stock, not
/// over an empty bank. The PLAF103's pinned baseline holds no bank 0 the
/// loader accepts, so its case shows only the selection.
pub fn installed_selection(current: &[u8], rom: &[u8], image: &[u8], model: Model) -> Result<()> {
    let payload = container(image)?;
    let fixture = placed(current, BANKS[1] as usize, payload)?;

    if model == Model::Plwf106 {
        let stock = factory::execute(&fixture, rom, Entry::Check(0))?;
        require(stock.stop == Stop::Returned && stock.result == 1, || {
            ReviewError::InstalledStock {
                stop: stock.stop,
                result: stock.result,
            }
        })?;
    }
    let chosen = factory::execute(&fixture, rom, Entry::Select)?;
    require(chosen.stop == Stop::Selected && chosen.result == 2, || {
        ReviewError::Installed {
            stop: chosen.stop,
            result: chosen.result,
        }
    })?;
    Ok(())
}

/// `current` with `payload` written at `start`, refused when it does not
/// reach that far.
fn placed(current: &[u8], start: usize, payload: &[u8]) -> Result<Vec<u8>> {
    let mut fixture = current.to_vec();
    let needed = start + payload.len();
    let Some(bank) = fixture.get_mut(start..needed) else {
        return Err(ReviewError::ShortFlash {
            bytes: current.len(),
            needed,
        }
        .into());
    };
    bank.copy_from_slice(payload);
    Ok(fixture)
}

/// Whether core `index`'s final mapping covers `length` bytes at flash
/// offset `physical` from `virtual_start`, subtractively.
fn maps(
    run: &FactoryRun,
    index: usize,
    virtual_start: u32,
    physical: usize,
    length: usize,
) -> bool {
    let Some(last) = run.configs.get(index * 2 + 1) else {
        return false;
    };
    let virtual_start = i64::from(virtual_start);
    let (Ok(physical), Ok(length)) = (i64::try_from(physical), i64::try_from(length)) else {
        return false;
    };
    i64::from(last.start) == virtual_start
        && i64::from(last.end) == virtual_start + length - 1
        && last.minus == 1
        && i64::from(last.offset) == virtual_start - 0x0800_0000 - physical
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    #[test]
    fn payload_must_be_a_valid_container_that_fits_the_aligned_bank() {
        assert!(bank_payload(b"OTA").is_err());
        assert!(bank_payload(&[0; 64]).is_err());
    }

    /// A manifest is bound to the image it freezes: its startup identity
    /// must be the one its own fields give, and the image must print it. An
    /// edited identity, or a manifest beside another variant's image, is
    /// refused.
    #[test]
    fn a_manifest_that_does_not_describe_the_image_is_refused() {
        let mut m = BuildManifest {
            device: "plaf103".into(),
            profile: "firstboot".into(),
            firmware: Some("feeder".into()),
            ..BuildManifest::default()
        };
        let derived = m.source_identity().unwrap();
        m.startup_identity = Some(derived.clone());
        let image = format!("code {} start build={derived} rest", Tag::Start).into_bytes();
        identity(&m, &image).unwrap();

        let printed_by_another = format!("code {} start build={} ", Tag::Start, "0".repeat(64));
        let error = identity(&m, printed_by_another.as_bytes()).unwrap_err();
        assert!(
            matches!(&error, Error::Review(ReviewError::NoIdentity { identity }) if *identity == derived),
            "{error:?}"
        );

        let mut pulse = m.clone();
        pulse.variant = Some("pulse".into());
        let error = identity(&pulse, &image).unwrap_err();
        assert!(
            matches!(&error, Error::Review(ReviewError::Identity { recorded: Some(recorded), .. }) if *recorded == derived),
            "{error:?}"
        );
    }

    #[test]
    fn stale_inputs_name_the_first_that_differs() {
        let recorded =
            BTreeMap::from([("a".to_string(), "1".to_string()), ("c".into(), "3".into())]);
        let mut current = recorded.clone();
        assert_eq!(first_difference(&recorded, &current), None);
        current.insert("b".into(), "2".into());
        assert_eq!(first_difference(&recorded, &current).as_deref(), Some("b"));
        current.remove("b");
        current.insert("c".into(), "4".into());
        assert_eq!(first_difference(&recorded, &current).as_deref(), Some("c"));
    }

    /// A flash read too short for the bank a case places the image in is
    /// refused, not a panic.
    #[test]
    fn a_short_flash_image_is_refused() {
        assert_eq!(
            placed(&[0; 8], 2, &[1, 2]).unwrap(),
            [0, 0, 1, 2, 0, 0, 0, 0]
        );
        let error = placed(&[0; 0x1000], 0x1000, &[1]).unwrap_err();
        assert!(
            matches!(
                &error,
                Error::Review(ReviewError::ShortFlash {
                    bytes: 0x1000,
                    needed: 0x1001
                })
            ),
            "{error:?}"
        );
    }

    #[test]
    fn an_unknown_profile_is_refused_before_anything_is_written() {
        let repo = Repo::here().unwrap();
        let private = repo.root.join("private");
        fs::create_dir_all(&private).unwrap();
        let scratch = tempfile::tempdir_in(&private).unwrap();
        let build = scratch.path().join("build");
        fs::create_dir(&build).unwrap();
        fs::write(build.join("manifest.json"), r#"{"profile": "bringup"}"#).unwrap();

        let output = scratch.path().join("output");
        let error = freeze(&repo, ImageKind::Build, &build, &output).unwrap_err();
        assert!(
            matches!(
                &error,
                Error::Review(ReviewError::Profile { profile, .. }) if profile == "bringup"
            ),
            "{error}"
        );
        assert!(!output.exists());
    }

    #[test]
    fn output_outside_private_is_refused() {
        let repo = Repo::here().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let output = outside.path().join("review");
        let error = freeze(&repo, ImageKind::Stock, Path::new("unused"), &output).unwrap_err();
        // A checkout without private/ (a fresh clone) is refused for that
        // first, before the output is read.
        assert!(
            matches!(
                error,
                Error::Review(ReviewError::Output)
                    | Error::Ota(wellfed_image::Error::Root(
                        wellfed_image::root::RootError::NoPrivate { .. }
                    ))
            ),
            "{error:?}"
        );
        assert!(!output.exists());
    }
}
