# Firmware builds

This guide, for contributors who already pass `make check` ([getting started](README.md)), builds a full firmware image with the vendor SDK and signs one as an update. Building never flashes hardware.

Host tests and cross-compilation checks need no vendor SDK. Full images use the pinned Realtek AmebaD SDK and ASDK compiler named in [sdk.lock.json](../../src/firmware/vendor/sdk.lock.json), a family-level reference, not a source match for stock's `sdk-amebad_v6.2C-RC`.

## Prerequisites

The build wrapper supports **Linux x86_64** and needs, installed by you (it installs no system packages):

- Git, Make, Bash, tar with bzip2, curl and the standard GNU tools;
- the pinned Rust toolchain;
- libclang, for bindgen;
- the 32-bit loader and runtime that Realtek's i686 compiler needs.

The builder is `wellfed-build` (`src/tools/wellfed-build`). The make targets build it and run it as its own executable, not through `cargo run`, which sets `RUSTUP_TOOLCHAIN`, which a firmware build refuses.

## Devices, firmwares and profiles

A build is a **profile** of a **firmware** on a **device**:

```sh
wellfed-build build --device plaf103 --profile firstboot [--firmware qa] [--variant fail]
make -C src/firmware/vendor firstboot DEVICE=plaf103 FIRMWARE=bringup VARIANT=fail
wellfed-build list    # the devices, and each firmware's devices and variants, as JSON
```

- **Device:** `src/firmware/devices/<name>/device.json` names the board's platform (the chip family) and the files that platform stages: the application entry and first-boot mains, and the verified stock image. Its model names the device's directory, which holds the model's [layout](../../src/firmware/devices/plaf103/layout.json), from which the `device` and `firstboot` profiles derive their banks and pad table. `--device` names the model (`plaf103`), with no default even for one board; the `make` targets pass `DEVICE=plaf103`. Every bench tool takes the same flag: the flash writers (which hold each model's pins, [Rust flash client](rust-flash.md#each-models-pins)), the audits, `wellfed-stock` (which picks its message set by it) and `wellfed-bench serve` (which passes it to each job's flash writer and build).
- **Firmware:** a crate under `src/firmware/wellfed/` ([firmware guide](../../src/firmware/wellfed/README.md)) whose `[package.metadata.wellfed]` table declares the devices it builds for, its normal first-boot image's startup policies, and its opt-in variants. A variant is the Cargo feature of the same name, and its policies replace the normal image's. A new firmware or variant needs no builder change.
- **Platform:** code, not configuration (`wellfed-build`'s `src/platform/`). Its SDK overlays anchor on exact text in the pinned SDK and its audits read disassembly, so both fail closed. AmebaD is the only one; another chip would be a new `Platform` variant and a module beside `amebad`.

### Profiles

Each profile adds stages to the one before.

| Profile | What it builds |
| --- | --- |
| `baseline` | The unchanged SDK example, for build comparison |
| `bringup` | The SDK example with the Rust application, STOP/off initialization, and a bounded indicator entry hook |
| `device` | The application with the device's stock-derived bank addresses and audited KM0 pin pulls. Needs the device's verified stock OTA |
| `firstboot` | Minimal UART-only SDK mains plus the application, Wi-Fi, the device API and updates, with the same stock need. The image the bench reviews and installs, and the only profile that takes a variant |

### Bench and consumer images

A Rust profile is built for the bench (the default) or a consumer: `--provisioning bench|consumer`, or `PROVISION=consumer` for make.

| | Bench | Consumer |
| --- | --- | --- |
| Build Wi-Fi | The hotspot in `private/hotspot.json`, generated into the entry's header | None: the header defines nothing, so the image joins only the network the owner stores |
| Built-in update key (`firstboot`) | The bench key's public half from `private/ota-key.json`, if there is one | The project release key's public half from `private/release-key.pub.json`; without it the build is refused, never falling back to the bench key |
| Output | `private/firmware-build/<device>/<profile>/` | `private/firmware-build/<device>/<profile>-consumer/` |

```sh
make -C src/firmware/vendor firstboot FIRMWARE=feeder PROVISION=consumer
```

The release key file holds only the public half:

```json
{"version": 1, "public_key": "<64 lowercase hex characters>"}
```

Any other field is refused, so a file that also holds a `seed` never reaches a build. The release key itself stays offline: `wellfed-build release-keygen` writes it and this file side by side on removable media ([the release key](#the-release-key)).

A manifest for the app's installer must be signed by this key's private half, with `wellfed-build sign --key <file>`. The default, `private/ota-key.json`, is the bench key, whose manifest a consumer image refuses at promotion once the handover from stock has cleared enrolled keys. For bench tests without the real release key, create a bench-only stand-in with `wellfed-build keygen --output private/release-standin-key.json` and put its public half in `private/release-key.pub.json`. `make app-installer-input` refuses a manifest this key did not sign, or one for another build.

The manifest records `provisioning`, which the source identity binds, so bench and consumer images of the same sources never share an identity; `source-id` and `inputs` take `--provisioning` too. The builder sets `WELLFED_BUILD_WIFI` exactly when the generated header names a network, and the image's Rust reads build Wi-Fi from its presence. The header defines a macro of the same name, and the entry reads a runtime symbol only the matching Rust build defines, so a disagreeing header and runtime fail the link. The loop reads the same answer (`Platform::build_wifi`): a consumer image with no Wi-Fi stored starts BLE setup mode at boot ([product architecture](../firmware/product-architecture.md#setup-and-pairing)).

A consumer image trusts the release key, not the bench key, so once installed the bench could offer it nothing more. `wellfed-bench ota` therefore refuses a consumer review unless the feeder already lists the bench key as an enrolled update signer: run `wellfed update-key enroll` with the bench key first.

## Build an image

1. Fetch the SDK and compiler:

   ```sh
   make -C src/firmware/vendor bootstrap
   ```

   You should see the SDK revision fetched and the compiler archive downloaded and checksummed into `private/`. No check target runs this for you.

2. Build the unchanged SDK example and the bring-up profile:

   ```sh
   make -C src/firmware/vendor baseline
   make -C src/firmware/vendor bringup
   ```

3. For the PLAF103's `device` and `firstboot` profiles, place your own exact stock 1.1.15 image at `private/firmware/PLAF103_OTA_010115.bin`, the path its `device.json` names; the layout file records its hash ([fixtures](fixtures.md)).

4. Build the device profiles:

   ```sh
   make -C src/firmware/vendor device
   make -C src/firmware/vendor firstboot
   ```

   You should see the build, then its audits, finish without a refusal. The manifest, logs and Rust archives land in `private/firmware-build/<device>/<profile>/`.

5. To freeze the image for installing, continue with [development updates](uart-development.md).

Builds run KM0 before KM4 and keep input and artifact hashes, logs and successful manifests. Bindgen gets the SDK and newlib paths from the wrapper through `WELLFED_SDK_DIR` and `WELLFED_NEWLIB_INCLUDE`.

### Where output goes

| Ignored location | Contents |
| --- | --- |
| `private/upstream/ameba-rtos-d/` | The pinned upstream checkout |
| `private/toolchains/` | The downloaded archive and extracted compiler |
| `private/firmware-build/<device>/<profile>/` | The generated SDK worktree, Rust archives, logs and manifest |
| `private/firmware-build/<device>/<profile>-consumer/` | The same, for a consumer image |

Change tracked source, never a generated SDK worktree: profiles stage tracked files and apply audited overlays.

## What the builder refuses

**Bootstrap.** The compiler archive digest was computed from the official release download; it is not an independent upstream signature.

- An existing SDK checkout at an unexpected revision is refused.
- The compiler is extracted into a temporary directory, checked and renamed into place. Only entries under its pinned directory are extracted, links must stay inside it, and special files are refused.
- An existing partial compiler tree is refused rather than reused.
- Every run verifies the pinned compiler and binutils file hashes in [toolchain.lock.json](../../src/firmware/vendor/probe/toolchain.lock.json).

**A dirty SDK worktree.** A reused worktree is refused, naming the paths, when `git status` shows anything beyond the profile's own overlays and the exact vendor-recipe outputs listed in `wellfed-build`'s `sdk.rs` (executable-bit changes with unchanged content, the rebuilt `lib_user.a`, toolchain links and two KM0 loader outputs). Nothing is cleaned automatically: review, then restore or recreate the worktree. Files the SDK's own rules ignore are not checked.

**Rust overrides.** Firmware builds refuse output-affecting overrides instead of silently dropping them:

- `RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS`, `CARGO_BUILD_RUSTFLAGS`, `RUSTC*`, `RUSTUP_TOOLCHAIN`, `CARGO_PROFILE_*`, `CARGO_UNSTABLE_*`, incremental settings and firmware-target `CARGO_TARGET_*_RUSTFLAGS`;
- the equivalent `build`, `profile`, `unstable` and firmware-target keys in applicable Cargo configuration files.

Host-only target settings, such as a host linker, are allowed. The manifest records the `rustc -vV` cargo resolves from the repository, which must match `rust-toolchain.toml`.

## What a build does and doesn't do

Builds package application images and audit symbols, layout, pin pulls and flash hooks where they apply. They do not flash, drive a motor, validate a physical board, or authorize replacing the factory loader. Vendor timestamps mean output is not guaranteed byte-for-byte reproducible.

The images do touch hardware when they run. Upstream example startup can touch it before the diagnostic, which configures STOP/off GPIO, restores shared-pad supply startup in the minimal KM0 main, and pulses the four indicators ([startup behavior and outstanding readback faults](../firmware/startup-diagnostics.md)).

> [!WARNING]
> A rebuilt UART diagnostic will generally differ from the exact artifact the retained recovery writer accepts. Never update its hash to get past review ([recovery restrictions](../devices/plaf103/recovery.md)).

## Source identity

Every build carries a source identity binding exactly what it was built from.

- `cargo run -q -p wellfed-build -- source-id --device plaf103` computes the current firstboot-profile source ID offline, without the SDK. `check-cross` supplies it to the SDK-independent KM0 port compile.
- `source-id --image` (with `--variant`, if any) prints the identity a build would embed instead, which also binds the variant and the update key.
- Every identity binds the device and profile names and the provisioning. `wellfed-bench` uses it to reuse a review of the same sources.

The Rust inputs are the Git-tracked and untracked, non-ignored files of:

- the SDK crates under `src/sdk/`;
- the product crates under `src/product/` they link;
- the selected firmware's crate (not `target` directories there, nor other firmwares or host tools);
- the device's `device.json` and the files it names that the profile stages;
- the builder's own `src/` and the two `wellfed-image` modules its layout audit shares.

So a changed recipe or audit is a new identity; a changed builder fixture test is not. Ignored editor or build files and the checkout's location don't change the ID, which is why `source-id` needs a Git checkout.

SDK builds freeze the input map, embed its real identity, and record clock, output, LED, routing and shared-pad policies for host verification. A compile-only check produces no reviewed installable image.

| Command | Does |
| --- | --- |
| `wellfed-build inputs --device plaf103 --profile PROFILE` | Prints the current input map |
| `wellfed-build audit --build DIR` | Re-runs a finished build's audits, as JSON |
| `wellfed-build review` | Checks a build against both before freezing it ([development updates](uart-development.md)) |
| `cargo test -p wellfed-build -- --ignored` | The builder's fixture tests, in `make check-fixtures`, which fail when an ignored build no longer matches the current sources |

## Update signing

The firmware installs an update only if its manifest is signed by the key the build embeds. Both commands run offline:

```sh
./target/release/wellfed-build keygen
./target/release/wellfed-build sign --review private/verification/<review> --version 2 \
  --output private/<name>.manifest
```

- **`keygen`** writes a new Ed25519 key to `private/ota-key.json` (mode 600), marked `"scope": "bench"`, and prints the public key first-boot builds embed, which joins their identity. It never replaces an existing key.
- **`release-keygen --output PATH`** writes the project release key onto offline media ([the release key](#the-release-key)).
- **`sign`** writes the version 2 manifest the device API takes (`wellfed_signed::offer`), naming the signing key's id (the bench key's by default) and the image's kind. `--version` must exceed the feeder's floor (the highest version it confirmed), or come with a downgrade statement.
- **`downgrade --manifest M --device HEX --floor N`** is a statement letting the feeder whose identity key is `HEX` install the version 2 manifest `M` while its floor is `N`. A version at or below the floor, or any stock image, needs one.
- **`revoke --key-id HEX`** retires an update key id for good, and **`add-key --public HEX`** fills the firmware's release slot ([OTA design](../firmware/ota-design.md#keys-and-anti-rollback)). Only a key built into the firmware signs these key statements: the bench key or the release key. `keygen` and `release-keygen` print the key id.

`wellfed-bench ota` links the same library and signs with `private/ota-key.json`.

### What `sign` refuses

`sign` signs only a directory under `private/` that `wellfed-build review` froze, verified exactly as `wellfed-flash` verifies a review before installing, and refuses:

- a directory without `review.json`;
- an `image.bin` or payload that differs from the review;
- a `build-manifest.json` that differs from its `manifest_sha256`;
- a stock review of anything but the pinned stock image;
- a build that is not the first-boot profile;
- a manifest without well-formed SDK and toolchain pins (those the reviewer checked against the lock it froze under, not the checkout's current lock);
- a manifest whose `OTA_All.bin` is not this image;
- missing startup, pad-table or mapping audits;
- an image that does not carry the reviewed startup identity.

The build ID comes from the verified manifest (for the stock image, the review's payload hash), never from the command line. An image whose build declares a startup motion policy signs only with `--attended-motion POLICY` naming that exact policy.

### The bench key

`keygen`'s key is the bench key: it lives in a checkout's `private/` and signs development images for the bench's own units only. Every key file names its `scope`. The signer also loads an owner's key (`"scope": "owner"`, written by `wellfed update-key new`), which a feeder trusts only once the owner enrolls it there ([OTA design](../firmware/ota-design.md#keys-and-anti-rollback)), which signs no key statement (`revoke` and `add-key` refuse it), and which no image embeds. It loads the release key (`"scope": "release"`) too. No other scope loads.

A bench build refuses an owner's key or the release key in `private/ota-key.json`: it embeds only the bench key from there.

### The release key

The release key signs everything a consumer image takes: its update manifests, and `revoke` and `add-key` statements. Every consumer image builds in its public half, and bank 0 keeps the build it was written with.

> [!WARNING]
> If you lose the release key, no beta feeder can take another update over the air. Each could then only be updated over UART, with the case open. If it leaks, anyone can sign updates those feeders install. Keep two copies, offline, in different places.

You make it once, on a machine with no network, onto removable media. Only its public half ever enters a checkout.

1. While you still have the network, build the tool: `cargo build --release --locked -p wellfed-build`. `release-keygen` needs no checkout, so you can also copy `target/release/wellfed-build` to another Linux machine and run it there.
2. Unplug the network: pull the cable and turn Wi-Fi off. `ping -c 1 1.1.1.1` should fail.
3. Insert a removable drive and note where it is mounted, for example `/run/media/$USER/KEY1`.
4. Write the key:

   ```sh
   ./target/release/wellfed-build release-keygen --output /run/media/$USER/KEY1/wellfed-release-key.json
   ```

   It prints both paths, the public key and the key id. Write the key id down on paper. The drive now holds `wellfed-release-key.json` (the key) and `wellfed-release-key.pub.json` (the public half). The key is written mode 600, but a FAT or exFAT drive, as most USB sticks come, ignores that and lets every local user read it while it is mounted: format the drives ext4 first, or keep them mounted only for these steps.
5. Insert a second drive and copy both files to it, then check the copies: `cmp /run/media/$USER/KEY1/wellfed-release-key.json /run/media/$USER/KEY2/wellfed-release-key.json`.
6. Copy only the public half into the checkout: `cp /run/media/$USER/KEY1/wellfed-release-key.pub.json private/release-key.pub.json`. Never copy `wellfed-release-key.json` itself anywhere but removable media.
7. Unmount and remove both drives, then reconnect the network. Keep the drives in two different places.

`release-keygen` refuses:

- a relative `--output` path;
- a path inside a Git working tree: it looks for `.git` in every directory above the path, as written and with its symlinks resolved;
- a key file or a `.pub.json` that already exists, so it never replaces a key;
- a directory that does not exist.

To sign a release, unplug the network again, insert one drive, and name the key:

```sh
./target/release/wellfed-build sign --key /run/media/$USER/KEY1/wellfed-release-key.json \
  --review private/<review> --version <N> --output private/<name>.manifest
```

`revoke` and `add-key` take the same `--key`. Unmount the drive before you reconnect.

#### A stand-in you already moved offline

If you made a key with `keygen` and moved it to offline media to serve as the release key, it says `"scope": "bench"`. Change that one word to `"scope": "release"` and nothing else:

```json
{"version": 1, "scope": "release", "seed": "…", "public_key": "…"}
```

The signer then loads it as the release key. Its public half in `private/release-key.pub.json` stays as it is. If you don't have that file, write it by hand, with the key file's `public_key`: `{"version": 1, "public_key": "<64 hex>"}`. Delete any copy of the key left in a checkout.

## Next steps

- [Development updates](uart-development.md): freeze a build and install it over UART.
- [Firmware guide](../../src/firmware/wellfed/README.md): write your own firmware.
- [Architecture](../firmware/README.md) and [third-party notices](../../THIRD_PARTY.md).
