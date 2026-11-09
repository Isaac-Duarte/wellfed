# Contributor and agent instructions

## Start here

- The working files live in the ignored `.agents/` folder, not in Git: `.agents/RESUME.md` (the bench handoff), `.agents/FIRMWARE_BOARD.md`, `.agents/FEATURE_BOARD.md`, `.agents/SERVER_BOARD.md` (server crates and capabilities), `.agents/PERFORMANCE_BOARD.md` (CPU time, latency, stack, RAM and code size) and `.agents/SERVER_SPEC.md` (the owner's server design decisions). These hold open work only: closed cards, older checkpoints and the former API design move to `.agents/archive/`. Tracked docs never link to them; the public summary is [the roadmap](docs/roadmap.md), and card ids stay out of tracked docs.
- Read `.agents/RESUME.md` when continuing firmware work, then the boards, [README.md](README.md) and the [development guide](docs/development/README.md). Write docs by the [writing guide](docs/development/writing.md). Use docs/chips/ (shared SoC facts), docs/devices/ (one folder per feeder model, its board and stock firmware) and docs/firmware/ (the replacement firmware) for hardware assumptions and component contracts, and [product architecture](docs/firmware/product-architecture.md) for the product the SDK, server and app are built toward, with [server docs](docs/server/README.md) for its contracts and crates.
- Read [bench hardware](docs/development/bench-hardware.md) before wiring, checking or flashing anything on the bench. The Wellfed Bench board's FEEDER UART header names the feeder's pins, so feeder TX goes to the board's TX (label to label, never crossed), and nothing connects to a feeder's 3V3.
- Keep RESUME's checkpoint and its ignored local evidence index current when bench state changes. Reconfirm physical state rather than assuming the handoff is live.
- Keep the three boards (SDK and hardware capability; owner-visible behavior and stock parity; the server) current when implementation, dependencies or acceptance evidence change, and update [the roadmap](docs/roadmap.md) when a capability's public status changes. Card status never authorizes physical operations.
- Label every finding as physical observation, static analysis or simulation. Host checks do not establish hardware reliability.

## Hard rules

- Work within this repository. A neighboring research archive is not a build dependency or a mutation target.
- Keep user changes and private evidence intact. Never copy credentials, real device identifiers, captures, backups, vendor binaries or decompiled artifacts into tracked files.
- Never weaken pinned recovery hashes, marker sequencing, preservation checks or no-retry behavior to make tests pass. Rebuilt diagnostics are not automatically compatible with the pinned writer.
- Build and test commands must not flash hardware, operate motors, alter routing or start persistent services. Those need an explicit user task and a reviewed device-specific procedure (or the authorization below).
- Do not infer authorization to commit, push, publish or resume bench operations from historical notes. Respect the contributor's Git identity.
- Use English for all outward-facing project content, including documentation, UI copy, code comments, commit messages, pull requests and issues.

## Rust conventions

Everything first-party is under `src/`. The Rust workspace is `src/sdk/` (the SDK every firmware builds on), `src/product/` (product policy shared by the firmware and the server, starting with `wellfed-feeder`), `src/client/` (the owned device API values of `wellfed-model`, the sans-IO `wellfed-client` and the tokio `wellfed-connect` every host client shares), `src/server/` (the owner's server, layered as the [server design](docs/server/design.md#crates) lays out), `src/tools/wellfed-*` and `src/tools/xtask` (host tools) and `src/firmware/wellfed/` (one crate per firmware); `src/firmware/vendor/` holds the AmebaD C ports, pins and image build, `src/web/` the site and the docs site, `src/app/` the Flutter Android app and `src/integrations/homeassistant/` the Home Assistant integration, with its PyO3 bridge crate. The [Rust standard](docs/development/rust.md) is the full rule set, with what enforces each rule; in short, the compiler enforces most of them. Every crate inherits `[workspace.lints]`, and `make check-rust` runs a second Clippy pass over libraries and binaries with `PRODUCTION_LINTS` (no `unwrap`, no narrowing `as`, no needless by-value parameters, no unreachable `pub`). `make check-deps` runs cargo-deny over licenses, advisories and sources (`deny.toml`), and tests run under cargo-nextest. A deliberate exception is an `#[expect(lint, reason = "…")]` at the narrowest scope. Keep existing crate names, public protocols and the C interface the AmebaD ports call (the `wellfed_*` exports `src/firmware/vendor` uses, and the KM0 park and hook symbols) unless the task requires a compatibility change; there is no general-purpose C ABI over the SDK. Every host tool is Rust; the Python tools were ported or retired by October 23, 2026, and `cargo xtask check-repository` refuses Python under `src/` outside the Home Assistant integration.

- **Typed data.** Model JSON with `#[derive(Serialize, Deserialize)]` structs and enums. Use `serde_json::Value` only for JSON that is genuinely opaque: hashed or replayed verbatim, or passed through unread.
- **Shared formats.** A file one crate writes and another reads gets its type in `wellfed-formats`, not a copy in each reader. Anything hashed goes through `wellfed_formats::canonical`, which fixes the bytes regardless of field order.
- **Enums, not strings or flags.** Closed sets are enums (with `clap::ValueEnum` for CLI options). Prefer a two-variant enum over a `bool` parameter whose meaning isn't obvious at the call site. Firmware states, faults and reasons are Rust enums with fixed discriminants: a record or report carries `code()`, and a byte read from flash becomes a variant only through `from_byte` at the decode boundary.
- **Errors.** Library crates expose a `thiserror` enum; binaries and CLI glue use `anyhow` with context. No `Result<_, String>` and no `Refused(String)`-style catch-all: each refusal is a variant carrying the values its message names, so a caller or test matches it rather than its text. Checks go through `wellfed_formats::require(condition, || Error::…)`; an error from a crate below is wrapped whole (`#[from]`), not copied into a message.
- **No panics on input.** No `unwrap`/`expect` outside tests unless it guards an invariant the code just established, with the reason in the message. Use `let … else`, `try_from` and `?`.
- **Narrow visibility.** Default to `pub(crate)`. Use `pub` only for what another crate or an integration test uses.
- **One helper, one place.** Search before writing hex, hashing, HMAC, report-writing or serial helpers; the workspace already has them.
- **Firmware.** Firmware crates are `no_std` staticlibs. `unsafe` belongs only at the C interface and MMIO edge, and each block gets a `// SAFETY:` comment. Changing `repr(C)` types, `#[no_mangle]` symbols, the park/hook RAM code or flash record layouts changes audited binaries: rerun the link, RAM-park and stack audits and keep their pins.
- **Readable code.** Separate the logical steps in a function body (setup, the work, the result) with a blank line, and break long match arms, tests and builders into paragraphs. Comments and `//!` docs explain why the code exists and how it fits the system (invariants, hardware or protocol constraints, ordering requirements), not what the next line does. They state the constraint itself, not the board card it came from: card history lives on the boards and in RESUME, and `cargo xtask check-repository` refuses card ids under `src/`.

## App conventions

The Android app in `src/app/` is the only Dart in the repository. Its protocol logic lives in Rust, so the app, the server and the CLI cannot disagree about what goes over the wire. The [Dart standard](docs/development/dart.md) is the full rule set, with what enforces each rule.

- **Dart is the UI and platform IO.** Screens, state, Android permissions, BLE, NSD discovery and joining a Wi-Fi network belong in Dart, through maintained plugins under a permissive license. Device API sessions over TCP run in Rust through `wellfed-connect`, the driver the CLI and the server use.
- **Rust is everything the feeder checks.** Noise handshakes, pairing, invites, request building, event-cursor sync, validation and the installer's steps live in the sans-IO `wellfed-client` crate, which takes bytes in and returns bytes out. Dart moves those bytes and never reimplements them: no Dart crypto, signing or wire codec.
- **Generated types.** Dart types for requests, events and settings are generated from Rust through `flutter_rust_bridge`, as the web's TypeScript types are. Never hand-write a Dart copy of a Rust type, and commit regenerated bindings with the Rust change that caused them.
- **Feeding safety.** Every feed and state change goes through the device API like any other client. The app holds no schedule or feeding logic of its own, and a retry never resends a request the core marked uncertain.
- **Lints.** `analysis_options.yaml` enables strict analysis; fix warnings rather than suppressing them, and give a deliberate `// ignore:` its reason on the same line.

## Web conventions

The site and the docs site in `src/web/` follow the [TypeScript standard](docs/development/typescript.md), the full rule set and its enforcement. In short:

- **Generated contracts.** Colors and spacing come from `assets/brand/tokens.json`. Never hand-write a hex value.
- **Boundaries.** Only an app's `src/lib/api` touches `fetch`, `EventSource` or `BroadcastChannel`. ESLint enforces it.
- **Dependencies.** Exact versions, a frozen lockfile, a 7-day minimum release age, no install scripts unless listed, and an allowed license (`pnpm run licenses`).
- **Strict and quiet.** Strict TypeScript with no `any`, non-null assertions or casts outside generated code; zero warnings from svelte-check, ESLint and Prettier; a deliberate lint disable carries its reason on the same line.

## Python conventions

The Home Assistant integration in `src/integrations/homeassistant/` is the only Python in the repository. The [Python standard](docs/development/python.md) is the full rule set, with what enforces each rule. In short:

- **Python moves values; Rust decides.** Sessions, invites, writes, settlement, release checks and installs live in the `wellfed-homeassistant` bridge. The integration reads its typed classes and never parses the wire or retries a write.
- **Generated stub.** `bridge/wellfed_homeassistant.pyi` is generated from the module and committed with the change that caused it; mypy strict checks the integration against it.
- **Translated and typed.** Ruff, `mypy --strict`, and a translation key on every name, state and error the user sees.
- **Releases.** Agents build and check the release wheels (`make homeassistant-wheels`) but never publish to PyPI, create repositories or push tags. The owner does those.

## Compatibility

Wellfed is in active development, and the research unit is the only device running it. **Never add backward compatibility.** Change a format in place and drop the old reader: on-flash records, server JSON and database rows, file shapes and old evidence. No migrations, no `#[serde(default)]` shims, no capability bits or new variants added only to keep an old peer working. When a stored format changes, the change records the bench step that clears it (erasing or resetting the store) instead.

This covers what this repository builds. Interop with things it does not build, and the safety pins, stay exact:
- the device API's update path, the one deliberate exception for something this repository builds: every update passes through bank 0, which keeps the build it was written with, so a client that drifted from it could never update that feeder again without a UART cable. The session's opening (`Open::Session`, the IK handshake under `PROLOGUE`, `Hello`, `Welcome`), framing, discovery, the update ops, the keepalive and their answers are frozen as major 1, and so are invite redemption (`Open::Invite`, XXpsk3, `Join`, `Joined`, `CreateInvite`) and recovery (`Open::Recovery` and the setup exchange with its code proof), all pinned by `wellfed-proto/tests/fixtures/update-v1.txt`, whose test only appends. So are the signed formats bank 0 checks (the version 2 manifest, the downgrade, key-add and revoke statements, Ed25519 signatures and key ids, and the bank limits and loader layout an image is checked against), pinned by `wellfed-core/tests/fixtures/signed-v1.txt`. A frozen enum only gains variants at its end; only a new major changes the frozen part, and every client keeps speaking major 1's for good. Everything else in the device API only grows from the public beta, since mixed builds stay in use: no field is added, removed, reordered or retyped, an enum gains variants only at its end, a changed op is a new op and a retired one is refused `Unavailable`. `wellfed-proto/tests/fixtures/api.json` stores the traced structure of every message and `tests/api.rs` refuses a change in place; each growth raises `REVISION`, the ordered API revision the welcome carries ([device API evolution](docs/firmware/device-api.md#evolution)). So an older reader keeps reading: a unit enum inside an answer travels as `Lenient` (an unknown index reads as unknown), a struct that will grow ends in a `Tail`, events and alert entries carry their length, an answer a client can't decode is `Unreadable` and a write answered so is uncertain and never resent, while `Refused(Unknown)` is a refusal and the request did not run, a new kind of device message is skipped, and a whole-replace write (`PutPlans`, `PutCalibration`) never gains a field. A feeder of another revision is served by every client; `Skew` only says which side to update, and no client offers a feeder an older build. A pre-revision build (a schema hash of `0x1_0000` or more, `Skew::Legacy`) is sent the frozen part alone;
- the stock firmware's wire protocol and on-flash formats;
- the frozen reviews, plans and journals `wellfed-flash` accepts, including those the former Python tools wrote;
- canonical JSON that is hashed;
- every pinned check in the hard rules.

## Checks

| Change | Run |
| --- | --- |
| Any Rust edit | `cargo fmt --all`, then `make check` (the contributor gate) |
| Builder, audits, formats or flash review | Also `make check-fixtures` against fresh builds (`make -C src/firmware/vendor device firstboot`, and `make -C src/firmware/vendor firstboot FIRMWARE=feeder PROVISION=consumer` for the consumer image) |
| Firmware crates | Also a fresh image build, so its link, RAM-park and stack audits run |
| `src/web/` | Also `make check-web` (needs Node and pnpm, runs no Rust): ESLint, Prettier, licenses, types, unit tests, the build, and Playwright of the site and the docs site |
| `Cargo.lock` or `deny.toml` | `make check` includes `check-deps` (cargo-deny); its first run downloads the RustSec database |
| `src/integrations/homeassistant/` | Also `make check-homeassistant` (needs uv): the bridge's Clippy with its PyO3 module, the committed stub unchanged, Ruff, mypy strict, the wheel build and Home Assistant's tests against the simulator with 95% coverage |
| `src/app/`, or a `wellfed-client` type the app uses | Also `make check-app` (needs the Flutter SDK): bindings regenerated with no diff, `dart format`, the lint plugin's tests, `dart analyze --fatal-infos`, `flutter test` with the goldens and the smoke tests, and a debug APK build |

A skipped or missing fixture is not a passing check.

## Owner authorization (October 22, 2026)

For an agent working an owner-assigned firmware task:
- It may flash any flash region through the reviewed bench tools, as many times as needed.
- For live tests it may bring the feeder hotspot up and run `wellfed-stock serve` (until October 26, `wellfed-local serve`), including OTA offers, as a bounded foreground test process.
- Afterwards it must restore the prior network and service state and record what ran.

This does not cover motor motion, host firewall or system-service changes, publishing outside this repository's remote, or weakening any pinned check above.
