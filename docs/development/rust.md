# Rust standard

The full rules for every Rust crate in the workspace, why each exists, and what enforces it; [AGENTS.md](../../AGENTS.md#rust-conventions) keeps the short form. A rule enforced by a tool is checked by `make check`; a rule marked **review** relies on whoever reads the change.

## Layout

| Folder | Holds |
| --- | --- |
| `src/sdk/` | The SDK every firmware builds on |
| `src/product/` | Product policy shared by the firmware and the server |
| `src/client/` | The device API values, the sans-IO client and the tokio connector every host client shares |
| `src/server/` | The owner's server |
| `src/tools/wellfed-*`, `src/tools/xtask` | Host tools ([inventory](../../src/tools/README.md)) |
| `src/firmware/wellfed/` | One `no_std` staticlib per firmware ([firmware guide](../../src/firmware/wellfed/README.md)) |

Every crate sets `license.workspace = true` and `publish.workspace = true`, and inherits `[workspace.lints]`. Every host tool is Rust: `cargo xtask check-repository` refuses Python under `src/`.

## Rules

| Rule | Why | Enforced by |
| --- | --- | --- |
| **No panics on input.** No `unwrap` in library or binary code, and `expect` only for an invariant the code just established, with the reason as its message. Use `let … else`, `try_from` and `?`. | A malformed file, packet or reply must become an error a caller can report, never a crash mid-flash or mid-feed. | `PRODUCTION_LINTS` (`clippy::unwrap_used`) in `make check-rust`; review for `expect` |
| **No silent narrowing.** `try_from` instead of a narrowing `as`. | Offsets, lengths and sector numbers that wrap write to the wrong place. | `PRODUCTION_LINTS` (`clippy::cast_possible_truncation`) |
| **Narrow visibility.** Default to `pub(crate)`; `pub` only for what another crate or an integration test uses. | The public surface is what a later change has to keep compatible. | `PRODUCTION_LINTS` (`unreachable_pub`) |
| **Borrow what you read.** No needless by-value parameters. | Avoids clones and moves that exist only to satisfy a signature. | `PRODUCTION_LINTS` (`clippy::needless_pass_by_value`) |
| **No leftovers.** No `dbg!`, `todo!` or `unimplemented!`. | Each is a debugging aid or a placeholder that panics at run time. | `[workspace.lints.clippy]` (`dbg_macro`, `todo`, `unimplemented`) |
| **Deliberate exceptions.** A lint exception is `#[expect(lint, reason = "…")]` at the narrowest scope. | `expect` warns when the exception stops being needed, and the reason says why it was. | Review. Older `#[allow]`s without a reason remain; convert them when you touch the code. |
| **Typed data.** JSON is `#[derive(Serialize, Deserialize)]` structs and enums; `serde_json::Value` only for genuinely opaque JSON, hashed or replayed verbatim, or passed through unread. | A field typo or a missing field fails at the parse, not three calls later. | Review |
| **Shared formats.** A file one crate writes and another reads has its type in `wellfed-formats`, and anything hashed goes through `wellfed_formats::canonical`. | Two copies of a type drift, and hashed bytes must not depend on field order. | Review |
| **Enums, not strings or flags.** Closed sets are enums (`clap::ValueEnum` for CLI options), and prefer a two-variant enum to a `bool` whose meaning isn't clear at the call site. Firmware states and faults are enums with fixed discriminants, not integer wrappers: `code()`, and `from_byte` where flash bytes are decoded. | The compiler checks every match when a variant is added. | Review |
| **Errors.** See [Errors](#errors) below. | Callers can match on what went wrong, and the operator reads a chain of context. | Review |
| **One helper, one place.** Search before writing hex, hashing, HMAC, report-writing or serial helpers. | The workspace already has them, and a second copy diverges. | Review |
| **`unsafe` at the edge only.** Host crates forbid `unsafe` at their roots. Firmware permits it only at the C ABI and MMIO boundary, each block with a `// SAFETY:` comment. `unsafe_op_in_unsafe_fn` is denied everywhere. Between Rust crates an interface is a trait, never an `extern "C"` table with a context pointer. | Every unsafe operation is visible and justified where it happens. A direct call between Rust crates lets the stack audit follow it, since it refuses a Rust indirect call outside `core::fmt`. | `#![forbid(unsafe_code)]` in host crates; `[workspace.lints]` (`unsafe_op_in_unsafe_fn`, `clippy::undocumented_unsafe_blocks`) |
| **Audited binaries.** Changing `repr(C)` types, `#[no_mangle]` symbols, the park/hook RAM code or flash record layouts reruns the link, RAM-park and stack audits and keeps their pins. | The pinned recovery path depends on them. | The image build's audits |
| **Readable code.** Blank lines between a function's logical steps; comments and `//!` docs explain why (invariants, hardware or protocol constraints, ordering), not what the next line does. | The reader is usually debugging a bench run. | `cargo fmt` for layout; review for the rest |

### Errors

- Library crates expose a `thiserror` enum; binaries and CLI glue use `anyhow` with context.
- No `Result<_, String>` and no `Refused(String)`-style catch-all: each refusal is a variant carrying the values its message names.
- Checks go through `wellfed_formats::require(condition, || Error::…)`.
- An error from a crate below is wrapped whole (`#[from]`), not copied into a message.
- A module with several refusals has its own enum named for it (`ProtocolError`, `PinmapError`), wrapped in the crate's `Error`.
- When a check's function returns the crate's error, build it in the closure (`|| ProtocolError::Interrupted.into()`), not with `Ok(require(..)?)`.
- Tests match the variant, not its text.

## Tests

`make check-rust` runs the suite with [cargo-nextest](https://nexte.st): each test in its own process, every failure reported in one run. A test still running after 60 s is named, then killed after 180 s ([.config/nextest.toml](../../.config/nextest.toml)). It runs twice as many tests at once as the machine has cores, since most wait on a socket or a timer (`make check-rust TEST_THREADS=n` sets another count), and starts the longest first. No doctests run: nextest skips them, and `cargo xtask check-repository` refuses a doc example rustdoc would compile, so an example is fenced `ignore` or `text`, and code that should run is a test.

- Tests may `unwrap`, `expect` and `dbg!` ([clippy.toml](../../clippy.toml)). A panic there is a failed test.
- Hardware and fixture evidence tests are `#[ignore]`d and run only from `make check-fixtures`, which stays on `cargo test` for its `--ignored --exact` selection. An ignored or skipped test is never a pass.
- Network tests bind loopback sockets with synthetic identities.
- The network parsers carry bounded [proptest](https://proptest-rs.github.io/proptest/) properties (no panic on any input, and encode-then-decode round trips) over the device API server and codec, the Noise handshake reader, BLE setup, and the update manifest and writer. They run in the ordinary suite, so `make check` covers them, and their drivers back the cargo-fuzz targets ([fuzzing](README.md#fuzzing)).
- Split a test file by behaviour once it passes about 1,500 lines. Keep it one binary: `tests/<name>/main.rs` declares a module per behaviour, and a `support` module holds the shared fakes and helpers. The binary keeps its name, so `cargo nextest run --test <name>` still works, and the build gains no binary. Each module opens with a `//!` line saying what it covers, so a failure's module path says where to look. Review enforces this; no check counts lines.
- Keep each firmware stage (a step of the loop or the settings writer) in a file of its own, one concern per file, so the file's name says where its code is. Review enforces this too.
- A proptest failure records its seed: commit it with the fix, in `<name>.proptest-regressions` beside a test under `tests/`, or in `proptest-regressions/<file>.txt` at the crate root for one in `src/` (proptest's default persistence).

## Dependencies

`make check-deps` runs [cargo-deny](https://embarkstudios.github.io/cargo-deny/) against [deny.toml](../../deny.toml):

- **Licenses:** permissive only (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode, MPL-2.0 and a few more), so every binary built from this MIT repository can ship. The one exception, the Unicorn emulator (GPL-2.0), is linked only by the host tool `wellfed-emu` and never reaches firmware.
- **Advisories:** vulnerable, unmaintained and yanked crates fail. An ignored advisory carries its reason; today only `bare-metal`, archived but still a dependency of cortex-m 0.7.
- **Duplicates** warn, not refuse. Several come from crates partway through an upgrade (digest 0.10 and 0.11, rand_core, syn 2 and 3); a change that adds one says why.
- **Sources:** crates.io only.

### First-party linkage

cargo-deny's `wrappers` keep the bench's hardware edge inside the crates that need it:

- `wellfed-relay` may be linked only from `wellfed-bench`, `wellfed-rig` and `wellfed-itest`.
- `wellfed-build` may be linked only from `wellfed-rig`, whose tests freeze a stock review; the rig signs through `wellfed-signing`.

The same wrappers keep the SDK below the product: `wellfed-device`, `wellfed-device-api`, `wellfed-device-runtime`, `wellfed-feeder`, `wellfed-feeder-runtime` and `wellfed-proto` are linked only by the product, firmware and host crates that use them today, so no `src/sdk` crate can take one on. The board crates (`wellfed-board-plaf103`, `wellfed-board-plwf106`) are reached only through the board contract, and `wellfed-hal-feeder` only from the feeder firmware, the bring-up probes, the virtual feeder and the feeder simulator ([deny.toml](../../deny.toml) lists each crate's linkers).

Some crates may not be linked at all. cargo-deny cannot ban every dependent without banning the crate itself, so `cargo xtask check-repository` refuses any manifest under `src/` that names them:

- `wellfed-flash`, the flash client, which every read and write runs as a process; the image, bank and review checks other crates share live in `wellfed-image`;
- `wellfed-stock`, the stock server.

`cargo xtask check-repository` also checks the server's layering from `cargo metadata`:

- Only `wellfed-store` depends on sqlx.
- Nothing below `wellfed-server` links axum.
- Each server crate takes only the workspace crates its arrows name.
- `wellfed-api` keeps to an allow-list, so it builds for Android ([server design](../server/design.md#allowed-dependencies)).

### The fuzz crate

`check-deps` then checks the out-of-workspace fuzz crate's lockfile against [src/fuzz/deny.toml](../../src/fuzz/deny.toml): the same rules, the allow list trimmed to that graph, and one exception, libFuzzer (NCSA, a permissive license), which `libfuzzer-sys` builds into host fuzzing binaries only.

### Offline and new dependencies

The advisory check reads the RustSec database, which the first run downloads; offline, `cargo deny check --disable-fetch` uses the cached copy. A check that could not run is not a pass.

The change that adds a dependency justifies it: what it replaces, or why the workspace can't do without it. It uses a `[workspace.dependencies]` version when one exists, and builds stay `--locked`.

## Tooling

You need Rust 1.99.1 through rustup ([rust-toolchain.toml](../../rust-toolchain.toml)) and `cargo install cargo-nextest cargo-deny`, plus `cargo-fuzz` for the optional [fuzz targets](README.md#fuzzing). Run `cargo fmt --all` after any edit, then `make check`.
