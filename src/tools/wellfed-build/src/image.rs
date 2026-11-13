//! One profile's build: bind the identity, hand the device's platform the
//! job, and write `manifest.json` from what it built.
//!
//! The manifest is removed first and written last, so a failed build never
//! leaves one describing other outputs. The platform re-runs [`Job::recheck`]
//! after compilation, because the vendor recipes take minutes and nothing
//! stops an editor touching the tree meanwhile.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use wellfed_formats::manifest::{BuildChecks, BuildManifest};
use wellfed_formats::require;

use crate::environment::{Rust, rust_compiler};
use crate::identity::{Subject, current_inputs, image_key, source_identity};
use crate::platform::{Job, Linked};
use crate::{Device, Firmware, Profile, Provisioning, Repo, Result, Stage, Variant};

/// Why a build request or a finished build was refused.
#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    #[error("Image variants apply to the firstboot profile only")]
    VariantProfile,
    /// Only a profile that links the application entry has a network or a
    /// key to provision.
    #[error("A consumer build needs a profile that links the Rust application")]
    ProvisioningProfile,
    /// Variants are test builds for the bench unit; none ships to a
    /// consumer, and the release key signs none.
    #[error("Variant {variant} is a bench build; a consumer build takes no variant")]
    ConsumerVariant { variant: String },
    /// The vendor recipes take minutes, and an editor may touch the tree
    /// meanwhile.
    #[error("Sources changed during the build; rerun before trusting these outputs")]
    SourcesChanged,
    #[error("Selected rustc changed during the build")]
    CompilerChanged,
    #[error("The build manifest names no firmware; rebuild it")]
    NoFirmware,
    /// Another build or review holds the profile's output directory. Every
    /// variant of a profile builds into the one directory, so two at once
    /// would record one's hashes beside the other's image.
    #[error("{} is in use by another build or review; wait for it", output.display())]
    Busy { output: PathBuf },
}

/// Hold `output` for this process alone until the returned handle drops: an
/// advisory lock every build and review of the directory takes, never
/// waited on.
pub(crate) fn hold_output(output: &Path) -> Result<fs::File> {
    let directory = fs::File::open(output)?;
    rustix::fs::flock(
        &directory,
        rustix::fs::FlockOperation::NonBlockingLockExclusive,
    )
    .map_err(|_| ImageError::Busy {
        output: output.to_path_buf(),
    })?;
    Ok(directory)
}

/// Remove a previous build's manifest before anything else, so a failed
/// build never leaves one describing other outputs.
fn remove_manifest(output: &Path) -> Result<()> {
    match fs::remove_file(output.join("manifest.json")) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}

/// What to build: a profile of a firmware (and variant) on a device,
/// provisioned for the bench or a consumer.
pub struct Request<'a> {
    device: &'a Device,
    profile: Profile,
    firmware: &'a Firmware,
    variant: Option<&'a Variant>,
    provisioning: Provisioning,
}

impl<'a> Request<'a> {
    /// Refused before any bootstrap or build work when the parts don't fit
    /// together.
    pub fn new(
        device: &'a Device,
        profile: Profile,
        firmware: &'a Firmware,
        variant: Option<&'a Variant>,
        provisioning: Provisioning,
    ) -> Result<Self> {
        require(variant.is_none() || profile.has(Stage::Firstboot), || {
            ImageError::VariantProfile
        })?;
        require(
            provisioning == Provisioning::Bench || profile.has(Stage::Rust),
            || ImageError::ProvisioningProfile,
        )?;
        if let (Some(variant), Provisioning::Consumer) = (variant, provisioning) {
            return Err(ImageError::ConsumerVariant {
                variant: variant.name().to_owned(),
            }
            .into());
        }
        if profile.has(Stage::Rust) {
            firmware.check_device(device.name())?;
        }

        Ok(Self {
            device,
            profile,
            firmware,
            variant,
            provisioning,
        })
    }
}

/// The progress line's description of a request.
fn describe(request: &Request) -> String {
    let mut line = format!("{} {}", request.device.name(), request.profile.name());
    if request.profile.has(Stage::Rust) {
        line.push_str(&format!(" firmware {}", request.firmware.name()));
    }
    if let Some(variant) = request.variant {
        line.push_str(&format!(" variant {}", variant.name()));
    }
    if request.provisioning == Provisioning::Consumer {
        line.push_str(" for consumers");
    }
    line
}

/// Build `request` with the toolchain the device's platform bootstrapped.
pub fn build(repo: &Repo, request: &Request, toolchain: &Path) -> Result<PathBuf> {
    let Request {
        device,
        profile,
        firmware,
        variant,
        provisioning,
    } = *request;
    // A consumer build without the release key is refused before it touches
    // its output. A bare firmware's image trusts no key (the build refuses
    // one that links the updater anyway).
    let ota_key = image_key(repo, profile, provisioning, firmware)?;
    let output = device.output(repo, profile, provisioning);
    fs::create_dir_all(&output)?;
    let _held = hold_output(&output)?;
    remove_manifest(&output)?;

    let rust = if profile.has(Stage::Rust) {
        Some(Rust::resolve(repo)?)
    } else {
        None
    };
    let expected = current_inputs(repo, device, profile, provisioning, firmware)?;
    let subject = Subject {
        device,
        profile,
        firmware,
        variant,
        provisioning,
    };
    let identity = source_identity(&subject, &expected, ota_key.as_deref())?;

    let recheck = || -> Result<()> {
        require(
            current_inputs(repo, device, profile, provisioning, firmware)? == expected,
            || ImageError::SourcesChanged,
        )?;
        if let Some(rust) = &rust {
            require(
                rust_compiler(repo, &rust.environment)? == rust.compiler,
                || ImageError::CompilerChanged,
            )?;
        }
        Ok(())
    };
    let job = Job {
        repo,
        device,
        profile,
        firmware,
        variant,
        provisioning,
        toolchain,
        log: output.join("build.log"),
        output: output.clone(),
        rust: rust.as_ref(),
        identity: &identity,
        ota_key: ota_key.as_deref(),
        inputs: &expected,
        recheck: &recheck,
    };
    println!("Building {}; log: {}", describe(request), job.log.display());
    let built = device.platform(repo).build(&job)?;

    let firstboot = profile.has(Stage::Firstboot);
    let rust_build = profile.has(Stage::Rust);
    let when = |condition: bool, value: &str| condition.then(|| value.to_string());
    let policies = firstboot.then(|| firmware.policies(variant));
    let policy = |pick: fn(&crate::firmware::Policies) -> &Option<String>| {
        policies.and_then(|p| pick(p).clone())
    };
    let compiler = rust.map(|rust| rust.compiler);
    let manifest = BuildManifest {
        device: device.name().into(),
        profile: profile.name().into(),
        sdk_revision: built.sdk_revision,
        startup_identity: when(rust_build, &identity),
        // The SDK runtime's startup, the same for every firmware.
        startup_output_policy: when(rust_build, "stop-only"),
        // The pulse probe is gone; startup only drives the indicators LOW.
        startup_led_policy: when(rust_build, "low-only-v1"),
        startup_gpio_routing_policy: when(rust_build, "read-only-v3"),
        startup_shared_pad_policy: when(firstboot, "before-km4-v3"),
        startup_gpio_clock_policy: when(rust_build, "explicit-enable-v1"),
        startup_flash_handshake_policy: None,
        startup_wifi_policy: when(firstboot, "wifi-gateway-v1"),
        // No build opens an MQTT session: status, requests and updates go
        // over the firmware's device API.
        startup_transport_policy: None,
        // Updates over the device API, checked against the built-in key.
        startup_update_policy: ota_key.as_ref().map(|_| "device-api-v1".into()),
        // What the firmware declares for its normal image or the variant.
        startup_adc_policy: policy(|p| &p.adc),
        startup_optics_policy: policy(|p| &p.optics),
        startup_motion_policy: policy(|p| &p.motion),
        startup_trial_policy: policy(|p| &p.trial),
        firmware: rust_build.then(|| firmware.name().into()),
        variant: variant.map(|v| v.name().into()),
        provisioning,
        ota_public_key: ota_key,
        toolchain_sha256: built.toolchain_sha256,
        built_at: Utc::now().to_rfc3339_opts(SecondsFormat::Micros, false),
        inputs: expected,
        sdk_overrides: built.sdk_overrides,
        compiled_archives: built.compiled_archives,
        rust_toolchain: compiler.as_ref().map(|c| c.version.clone()),
        rust_compiler: compiler,
        toolchain_files: built.toolchain_files,
        audits: built.audits,
        bootloader_replacement_supported: false,
        artifacts: built.artifacts,
        checks: BuildChecks {
            ota_payload_matches: true,
            ota_checksum_matches: true,
            bringup_banner_present: built.banner,
            required_core_symbols: built.required_symbols,
            source_snapshot_unchanged: true,
            sdk_worktree_within_allowlist: true,
            staged_sources_without_diagnostics: true,
        },
        device_compatibility: "Unverified SDK development image; desktop build only".into(),
    };

    let path = output.join("manifest.json");
    fs::write(&path, serde_json::to_string_pretty(&manifest)? + "\n")?;
    println!("Build complete; manifest: {}", path.display());
    Ok(output)
}

/// Re-run a finished build's audits for its review: the device, firmware
/// and archives come from its manifest, everything else from the build
/// directory and the checkout.
pub fn audit_build(repo: &Repo, build: &Path) -> Result<Linked> {
    let manifest: BuildManifest = wellfed_formats::load(&build.join("manifest.json"))?;
    let Some(name) = manifest.firmware.as_deref() else {
        return Err(ImageError::NoFirmware.into());
    };
    let device = Device::new(repo, &manifest.device)?;
    let firmware = Firmware::new(repo, name)?;
    let archives: Vec<(String, PathBuf)> = manifest
        .compiled_archives
        .into_iter()
        .map(|archive| (archive.core, build.join(archive.path)))
        .collect();

    device
        .platform(repo)
        .audit(repo, &device, build, &archives, &firmware)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    /// Two builds of one output directory, say two variants of a firmware,
    /// never run at once: the second is refused, and the directory is free
    /// again once the first lets go.
    #[test]
    fn a_held_output_directory_refuses_a_second_build() {
        let output = tempfile::tempdir().unwrap();
        let held = hold_output(output.path()).unwrap();

        let error = hold_output(output.path()).unwrap_err();
        assert!(
            matches!(&error, Error::Image(ImageError::Busy { output: busy }) if busy == output.path()),
            "{error:?}"
        );

        drop(held);
        hold_output(output.path()).unwrap();
    }

    /// A variant is a bench test build: asking for one for consumers is
    /// refused before any build work, and the same variant builds for the
    /// bench.
    #[test]
    fn a_consumer_build_takes_no_variant() {
        let repo = Repo::here().unwrap();
        let device = Device::new(&repo, "plaf103").unwrap();
        let firmware = Firmware::new(&repo, "feeder").unwrap();
        let variant = firmware.variant("virtual-panel").unwrap();

        let refused = Request::new(
            &device,
            Profile::Firstboot,
            &firmware,
            Some(&variant),
            Provisioning::Consumer,
        )
        .err();
        assert!(
            matches!(
                &refused,
                Some(Error::Image(ImageError::ConsumerVariant { variant })) if variant == "virtual-panel"
            ),
            "{refused:?}"
        );

        for (variant, provisioning) in [
            (Some(&variant), Provisioning::Bench),
            (None, Provisioning::Consumer),
        ] {
            Request::new(
                &device,
                Profile::Firstboot,
                &firmware,
                variant,
                provisioning,
            )
            .unwrap();
        }
    }
}
