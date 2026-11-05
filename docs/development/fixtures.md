# Optional fixtures

Some tests check the code against real binaries: the stock image, the SDK and the research unit's recordings. This page says what those inputs are and where they go. Ordinary contributor checks need none.

## Run the suite

```sh
make check-fixtures
```

The suite fails on a missing fixture, an incompatible hash, an error or a skip. It downloads nothing and produces no missing evidence, and a passing synthetic suite says nothing about these checks. Evidence tests are Rust tests marked `#[ignore]`, which plain `cargo test` lists as ignored, not passed, so report results from the Make target.

The emulation, review and flash-client evidence tests (`wellfed-emu`, `wellfed-build`'s `tests/review.rs` and `wellfed-flash`'s `tests/emulated.rs`) read their inputs from the checkout they were built in. A Git worktree has no evidence, so set `WELLFED_FIXTURE_ROOT` to another checkout to use its `private/`, where their temporary outputs then go and are removed afterwards.

The update-signing tests of `wellfed-build` and `wellfed-bench` freeze the pinned stock OTA (`private/firmware/PLAF103_OTA_010115.bin`) and sign it as `wellfed-bench ota` would, since only that exact image may be signed as stock.

## Where inputs go

| Location | What it holds |
| --- | --- |
| `private/firmware/PLAF103_OTA_010115.bin` | The stock 1.1.15 OTA you obtained yourself. The layout file records its exact hash |
| `private/upstream/ameba-rtos-d/` | The SDK checkout from an explicit bootstrap, which supplies candidate ROM and helper inputs |
| `private/toolchains/`, `private/firmware-build/` | SDK build artifacts and audited tools. A build whose manifest inputs differ from the current sources is stale, and its tests fail until you rebuild. Both `firstboot` and `firstboot-consumer` are needed: the consumer test scans the consumer OTA for the hotspot's SSID and passphrase and the bench key, and the bench OTA for them as a control, never printing them |
| `private/fixtures/baseline.bin` | The exact historical 8 MiB research baseline the pinned recovery checks require |
| `private/fixtures/current-read/flash-read.bin` | A recorded current full snapshot, into which the first-boot mapping test places the build |
| `private/fixtures/rom-inspection/report.json` | A finalized ROM report, with its linked serial evidence beside it |
| `private/fixtures/development-source/`, `development-target/` | Intact frozen review bundles for the helper-model and CLI tests |
| `private/fixtures/development-bounded-read/report.json` | The bounded-read evidence that goes with them |

Keep each capture or report bundle intact, and pass explicit report arguments when a test asks. Two independently captured bundles stay in separate subdirectories, each with its companion files. A directory alias must keep the companion serial and read files; aliasing one report file is not enough.

If you import your own private archive, ignored local symlinks can connect these locations to unchanged original bundles. The public checkout has none, and default tests don't need fixtures. Existing SDK artifacts may not validate against changed Wellfed inputs: rebuild and freeze a new target instead of rewriting old evidence.

## What these paths don't prove

These are test conventions, not proof of a valid session, and they don't make the research unit's exact recovery profile portable: nobody else is expected to have its baseline or historical artifact.

Get stock updates from your own authorized device and app, original backups from your own hardware, and the SDK and toolchain through the pinned upstream downloads. Those support your own analysis; a new device's backup will intentionally fail a writer pinned to the original research profile.

The historical baseline, captures and exact accepted diagnostic are not publicly distributed, so the full reference suite can't be reproduced from public files. For fixture-independent validation, run `make check`.

> [!WARNING]
> Never remove provenance checks or substitute hashes to get a green optional suite. To support another unit, design and review per-device compatibility and preservation criteria as separate work.
