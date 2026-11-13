# Getting started

This guide takes you from a fresh checkout to a passing `make check`, then shows which tool does what, how to work on one component, and how to run the simulator and the fuzzers. Nothing here needs or touches a feeder.

**Start here:**

1. Install the [prerequisites](#prerequisites) and [set up](#set-up) until `make check` passes.
2. Find your area:
   - code: the [Rust](rust.md), [Dart](dart.md), [TypeScript](typescript.md) or [Python](python.md) standard, then [work on a component](#work-on-a-component);
   - a host tool: [which tool do I use?](#which-tool-do-i-use);
   - a firmware image: [firmware builds](firmware-builds.md);
   - the bench: [bench hardware](bench-hardware.md) before any wiring, then the [relay bench](../devices/plaf103/bench.md);
   - docs: the [writing guide](writing.md).
3. Run `make check`, plus the check target for your area (`check-app`, `check-web` or `check-homeassistant`), before you hand a change over ([contribution guidance](../../CONTRIBUTING.md)).

## Prerequisites

We develop on Linux; `make check` also passes on macOS ([what differs](#macos)). You need:

- Rust **1.99.1** through rustup. [rust-toolchain.toml](../../rust-toolchain.toml) pins it with rustfmt, Clippy and the four ARM targets, which rustup fetches on first use.
- `cargo-nextest` and `cargo-deny`: `cargo install cargo-nextest cargo-deny`.
- GNU Make, CMake and a C compiler, with which the `unicorn-engine` crate builds the Unicorn emulator library. On Linux it also links libatomic.
- Permission to open loopback sockets, which network tests bind with synthetic identities.

Full firmware images also need the pinned vendor SDK and its compiler ([firmware builds](firmware-builds.md)), and the Android app its own [toolchain](../../src/app/README.md#toolchain).

## Set up

1. Clone the repository and open its root.
2. Run `cargo build --locked`. Rustup and Cargo download the toolchain and dependencies the first time. You should see the build finish with no errors.
3. Run `make check`. You should see formatting, Clippy, the tests, the cross-builds, the simulator and the repository checks pass. The first `check-deps` run downloads the RustSec advisory database.

If your environment forbids local sockets, give the network tests permission. Skipping them is not a passing result.

## macOS

`make check` runs on an Apple Silicon Mac with the same prerequisites, CMake and a C compiler coming from Homebrew and the Xcode command line tools. None of this is the desktop app, which is on the [roadmap](../roadmap.md).

The simulator, the `wellfed` CLI's device API commands, and `wellfed-bench` with its lease and console work as on Linux. These still need Linux:

| What | Why | On macOS |
| --- | --- | --- |
| `wellfed setup` | Its macOS Bluetooth, through CoreBluetooth, has not run against a feeder | It builds and runs, unproven. macOS asks for Bluetooth access for your terminal, and names the feeder by a UUID instead of its address |
| Finding the feeder in the hotspot's neighbour table | It uses `ip neigh` | Pass the feeder's address: `wellfed-bench serve --feeder-address` |
| The bench hotspot | It is a NetworkManager connection | Not available |
| Full firmware images | The vendor's compiler is a Linux x86_64 binary ([firmware builds](firmware-builds.md)) | `wellfed-build` refuses on another host; `make check`'s cross-builds still run |

A Mac's `make check` proves less than Linux's, so run it on Linux too before relying on what these cover:

- Ten tests are ignored there: they bind `127.x` addresses other than `127.0.0.1`, which only Linux's loopback answers without setup.
- The tests in `wellfed-bench`'s `tests/hangup.rs` build only on Linux, since they read each thread's state from `/proc`.

Under `nohup`, macOS doesn't show that SIGHUP is ignored, so `wellfed restore-stock` stops when its terminal closes, with its usual bounded stop.

## Repository layout

| Folder | Contents |
| --- | --- |
| `src/sdk/` | The SDK every firmware builds on: runtime, feeding core, hardware ports and board map ([firmware architecture](../firmware/README.md)) |
| `src/product/` | Product policy shared by the firmware and the simulators: the device protocol and the feeder's stores |
| `src/client/` | The clients the CLI, the Android app and Home Assistant share, and the libraries host programs share: file formats, image checks, signing and stock's protocol |
| `src/sim/` | The simulated devices, which the tools and other crates' tests run ([simulators](../../src/tools/README.md#simulators)) |
| `src/tools/` | Host tools, including the `wellfed` CLI ([tool list](../../src/tools/README.md)) |
| `src/firmware/wellfed/` | One crate per firmware: `feeder`, `qa`, `bringup` and `blink` ([firmware guide](../../src/firmware/wellfed/README.md)) |
| `src/firmware/vendor/` | The vendor SDK side: C hooks and ports, pinned SDK and toolchain locks, the stock layout, simulator scenarios and the image build (`make -C src/firmware/vendor`) |
| `src/firmware/devices/` | Per-device descriptions and layouts |
| `src/firmware/bench/` | The bench board's RP2040 firmware and its relay protocol crate ([bench firmware](../../src/firmware/bench/README.md)) |
| `src/hardware/bench/` | The Wellfed Bench board: KiCad project, fab outputs and enclosure ([bench board](../../src/hardware/bench/README.md)) |
| `src/web/` | Web frontends (a pnpm workspace): the [well-fed.dev](../web/site.md) site, the [docs site](../web/docs-site.md), and shared tokens and components |
| `src/fuzz/` | Fuzz targets ([fuzzing](#fuzzing)) |
| `assets/` | Brand marks, design tokens, the feeder's 3D model and its renders ([assets](../../assets/README.md)) |
| `docs/` | Hardware findings, firmware design and contributor guides |
| `private/` | Ignored, contributor-owned secrets, evidence, dependencies and artifacts |

The existing `wellfed-*` crate names are kept for compatibility.

## What checks do and don't touch

The check targets never fetch an SDK, contact a device, change firewall rules or start a persistent service, and network tests use loopback only. `check`'s targets install nothing; its first `check-deps` downloads the RustSec advisory database. `check-web`, `check-app` and `check-homeassistant` install their pinned dependencies (pnpm, pub, uv) into ignored folders, from the network on a first run. Build output goes to `target/`; the simulator worker and temporary verification outputs may create ignored folders under `private/`. You need no existing backup or evidence. [.gitignore](../../.gitignore) is the storage boundary: keep secrets out of the tracked examples.

The repository is self-contained: SDK and toolchain copies and fixtures resolve locally. The original research documents, tools and history sit in the ignored `private/research-archive/` ([provenance](provenance.md)). An evidence path in an old file is a record, not an instruction to open another checkout.

## Check targets

| Target | What it covers |
| --- | --- |
| `check-rust` | Formatting, Clippy with warnings denied, workspace host tests under cargo-nextest |
| `check-deps` | cargo-deny: licenses, advisories, duplicate versions and sources ([deny.toml](../../deny.toml)), then the fuzz crate's and the bench board firmware's lockfiles under their own policies ([fuzz](../../src/fuzz/deny.toml), [bench board](../../src/firmware/bench/wellfed-bench-board/deny.toml)) |
| `check-fuzz` | Clippy over the [fuzz targets](#fuzzing) on the pinned compiler, so each still compiles though only `cargo xtask fuzz` runs them |
| `check-cross` | Clippy with `PRODUCTION_LINTS` and a release build of the KM0 core's library for both boards, for the target, and `check-bench-board`: Clippy and a release build of the [bench board firmware](../../src/firmware/bench/README.md) for the RP2040. The KM4 crates need the SDK's bindings, so only an image build lints and compiles them |
| `check-virtual-feeder` | [The simulator](#the-simulator) alone. `check-rust` already runs it, so `check` does not run it again |
| `check-docs` | Local Markdown links and anchors, the commands, make targets and paths the docs name, status words and time-bound wording ([rules](writing.md#what-the-checks-refuse)), portability checks, no Python under `src/`, and the source rules (raw output writers, card ids). It builds only `xtask`, and runs no tool |
| `check-fixtures` | The optional binary and SDK evidence suite. Missing inputs or skips fail ([fixtures](fixtures.md)) |
| `check-web` | The site and the docs site: ESLint, Prettier, licenses, types, unit tests, the build and Playwright. Needs Node and pnpm, and runs no Rust ([TypeScript standard](typescript.md)) |
| `check-app` | The Android app: its generated bindings and theme tokens unchanged, `dart format`, the lint plugin's tests, analysis, widget, accessibility, golden and smoke tests, and a debug APK. Needs the pinned Flutter and the Android SDK ([Dart standard](dart.md)) |
| `check-homeassistant` | The Home Assistant integration: its bridge's Clippy with the Python module, its committed stub unchanged, Ruff, mypy strict, a wheel built and installed into a pinned Home Assistant venv, and its tests against the simulator with 95% coverage. Needs uv ([Python standard](python.md)). `make homeassistant-wheels` cross-builds the release wheels |
| `check` | Every contributor check except `check-fixtures`, `check-web`, `check-app` and `check-homeassistant` |

- `check-cross` proves portable compilation, not the SDK-linked hardware ports.
- Evidence tests are `#[ignore]`d in the ordinary suite; only `check-fixtures` selects them. An ignored test is not passing evidence.

## Which tool do I use?

Every host tool is Rust. [src/tools/README.md](../../src/tools/README.md) lists them all, with which touch hardware. The files they pass to each other are typed once, in `wellfed-formats`, and the bench's protocol in `wellfed-bench-formats`.

| I want to… | Use | Read |
| --- | --- | --- |
| Pair with a feeder, feed, plans, settings, history and status | `wellfed` | [commands](../../src/tools/README.md#the-wellfed-command-line), [device API](../firmware/device-api.md) |
| Run against a feeder without hardware | `wellfed-feeder-sim` | [simulators](../../src/tools/README.md#simulators) |
| Build a firmware image | `wellfed-build` | [firmware builds](firmware-builds.md) |
| Freeze a build for installing, or sign it as an update | `wellfed-build review`, `sign` | [development updates](uart-development.md) |
| Read or write the feeder's flash over UART | `wellfed-flash` | [Rust flash client](rust-flash.md) |
| Drive the relay bench: power, loader, OTA, the suite | `wellfed-bench` | [relay bench](../devices/plaf103/bench.md), [bench server](../devices/plaf103/bench-server.md), [integration suite](../devices/plaf103/integration-suite.md), [bench hardware](bench-hardware.md) |
| Serve stock 1.1.15 for an install | `wellfed-stock` (`wellfed-stock-server`) | [stock server](stock-server.md) |
| Audit a capture or compare full-flash backups | `wellfed-audit` | [tools](../../src/tools/README.md) |
| Test feeding logic against a modeled mechanism | `wellfed-sim` | [the simulator](#the-simulator) |
| Run the repository checks or a fuzzer | `cargo xtask` | [fuzzing](#fuzzing) |

## Work on a component

Run these from the repository root after an edit, then `make check` before you hand the change over:

```sh
cargo fmt --all
make check-rust
make check-virtual-feeder
```

## The simulator

The simulator runs the real Rust feeding owner, controller and ports against fake SDK inputs and fake NOR storage, with no feeder. It uses the PLAF103 firmware's feeding limits (`wellfed_sim::plaf103`), which the firmware's tests hold equal to `feeding::CONFIG`.

### The modeled mechanism

The plant (`wellfed_sim::plant`) is the mechanism measured on real feeds ([motor and feeds](../firmware/motor-and-feeds.md)):

- The mark input, PA18, rests HIGH and reads LOW for about 1.19 s at each mark.
- The next portion's mark comes about 9.07 s of drive after the last; with food in the hopper, about 9.8 s.
- The start-up surge lasts one poll over the current limit.
- While driving, the motor current averages near 60 (scaled), with peaks to about 167, as the empty feeds read.
- The rotor keeps its position across stops, power cuts and restarts.

A scenario's `plant` object can change the seed, the jitter, the rates of missed and bouncing edges, or the period. A `wedge` stalls the rotor going forward at the plant's stall current, which is unmeasured, and is either freed by the first reversal or not ([jams](../firmware/jams-and-sensor-faults.md)). The worker also stands in for the loop's claim on the outputs, and plays a background store writer that takes the write turn and a real flash lease (`write`).

### What it checks

Scenarios cover command IDs, sensor faults, a blocked outlet, missing and bouncing feedback, watchdog stalls and interrupted journal writes. Two liveness invariants hold across them:

- After a power cut anywhere (at each journal mutation of a reservation, result or resolution, and at moments through a feed), a new feed ID is accepted and completes.
- A feed completes, and every store write lands, with writes before, during and after it.

> [!NOTE]
> This is simulation evidence, not hardware reliability. It does not model all electrical behavior. A double bowl's mechanism is unmeasured, so its scenarios set their own marks.

### Run a scenario

Example scenarios are in [src/firmware/vendor/sim/scenarios](../../src/firmware/vendor/sim/scenarios).

```sh
cargo run -p wellfed-sim --bin virtual-feeder -- --scenario FILE --report NEW-FILE
```

You should see the report written to `NEW-FILE`. `make check-virtual-feeder` runs the whole set.

## Fuzzing

The network parsers have [cargo-fuzz](https://rust-fuzz.github.io/book/cargo-fuzz.html) targets in [src/fuzz/](../../src/fuzz/Cargo.toml), a crate outside the workspace with its own lockfile. `make check` lints them (`check-fuzz`) but never fuzzes.

1. Run `cargo install cargo-fuzz` once. libFuzzer also needs a C++ compiler.
2. Run one target for a bounded time (`--seconds` defaults to 60):

   ```sh
   cargo xtask fuzz api-connection --seconds 60
   ```

   It uses the pinned toolchain with `RUSTC_BOOTSTRAP=1` for libFuzzer's unstable compiler flags, so you need no nightly.

| Target | Input |
| --- | --- |
| `api-connection` | Any bytes on a new device API connection: the clear opening frame, the IK, invite and recovery handshakes, and a full session table |
| `api-session` | Requests up to the frame's limit, sealed inside the owner's open session, each answer opened and decoded |
| `proto-decode` | Every device protocol and setup message type, decoded and re-encoded |
| `noise-responder` | The first handshake message of each Noise pattern, into any payload buffer; only XX's may be read |
| `setup` | BLE setup messages to the feeder's responder, and a genuine client's request, code proof and Wi-Fi, held to the stores' rules |
| `ota-manifest` | The version 2 update manifest an offer carries (the bank-1 writer is held by its property tests) |

Builds, grown corpora and crash inputs stay in the ignored `src/fuzz/target/`, `src/fuzz/corpus/` and `src/fuzz/artifacts/`. On disk a target's name uses underscores (`api_connection`), as `src/fuzz/Cargo.toml` lists it. The device API and setup targets share their drivers with the property tests ([support](../../src/product/wellfed-device-api/tests/support/mod.rs)).

A crash is a parser bug:

1. Replay it with `cargo xtask fuzz <target> --replay src/fuzz/artifacts/<target_name>/<file>`.
2. Fix the parser.
3. Add the input as a regression test beside the property that should have caught it.

## Next steps

- [Firmware builds](firmware-builds.md): build a full image with the vendor SDK.
- [Optional fixtures](fixtures.md): run the evidence suite.
- [Stock server](stock-server.md): the server stock installs from.
- [Roadmap](../roadmap.md): what works today.
- [Glossary](../glossary.md): the terms these pages use.
