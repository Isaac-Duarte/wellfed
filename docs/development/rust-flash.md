# Rust flash client

`wellfed-flash` ([src/tools/wellfed-flash](../../src/tools/wellfed-flash)), the only flash client, reads and writes flash over UART on the PLAF103 feeder and the PLWF106 fountain: bounded reads, region-scoped [development updates](uart-development.md), give-ways and original-stock recovery. This page is for developers at the [relay bench](../devices/plaf103/bench.md).

It keeps the pinned helper, ROM and baseline checks, marker-last sequencing and terminal no-retry stops. Its reports, plans and journals keep the fields the former Python clients wrote, so evidence those clients saved still reads. A passing check here does not authorize a device session; any session needs a reviewed bench procedure.

## Build it

1. Run `cargo build --locked -p wellfed-flash`. You should see `target/debug/wellfed-flash` (`--release` builds `target/release/`).
2. Run it from the repository root, or pass `--root` to name the checkout whose ignored `private/` holds the evidence. All inputs and outputs must resolve inside that checkout.
3. Run `wellfed-flash <command> --help` to list a command's arguments.

## Commands

| Command | Does |
| --- | --- |
| `wellfed-flash read observe\|inspect-rom\|small\|full\|status\|press` | Restricted reads that never program or erase flash. `small`, `full` and `status` take `--device <model>`, and refuse a ROM report of another model's console. `press` types one press to a running `virtual-panel` build and waits for its echo ([virtual panel](../devices/plaf103/bench.md#virtual-panel)) |
| `wellfed-flash update --device <model> inspect\|prepare\|execute` | A region-scoped [development update](uart-development.md) of bank 1 (default) or bank 0. Flags: `--bank`, `--interrupted`, `--restore-marker`, `--original-read` |
| `wellfed-flash give-way --device <model> inspect\|prepare\|execute` | Clears bank 1's first marker byte so the loader falls back to bank 0 |
| `wellfed-flash stock-recovery --device <model> prepare\|execute` | Exact original bank-1 stock recovery from a full read |
| `wellfed-flash plan-stock-recovery` | Offline: plans stock recovery sectors from a baseline and a current read |
| `wellfed-flash decode-status` | Offline: decodes GD25Q64E status registers and assesses a write range |
| `wellfed-flash inspect-journal` | Offline: checks a transaction journal's hash chain; never resumes it |

A bounded `read small` uploads the pinned helper to a loader you have already entered, and later sessions continue that helper session. The relay bench, [`wellfed-bench`](../devices/plaf103/bench.md), runs this client as a separate process, keeping every plan bound to the `wellfed-flash` binary itself.

## How a write works

Every write takes the same four steps:

1. Run the command's `inspect`, which reads the device and writes nothing. Stock recovery has none: it prepares from a full read.
2. Run its `prepare`, which opens no port, writes the plan and prints its SHA-256.
3. Check the plan: the bank, the reviews it names, and the sectors it touches. A plan binds the SHA-256 of the binary that prepared it, so a rebuilt binary must prepare afresh.

   > [!WARNING]
   > `execute` erases and programs flash. Make a full backup first, and run it only against the unit and plan you just checked.

4. Run its `execute` with exactly that plan and hash.

### Frozen reviews

Frozen image reviews come from `wellfed-build review` ([development updates](uart-development.md)), which re-runs the build's linked-code audits and the factory-loader emulation; this client only checks that a review is complete and still matches its image. `wellfed-formats` types the review file for both.

The reviewer compared the build's SDK revision and toolchain digest with the lock of the checkout that froze it, and the review binds them through the manifest's hash. Using a review reads nothing from the current checkout's sources or locks, so an SDK, toolchain or `Cargo.lock` bump never strands an older review, bank 0's recovery image among them; the image hashes and every writer check are unchanged.

Each model has its own review kind, so a feeder review never installs on the fountain, nor the reverse. `wellfed-build review` writes the feeder's kind only, so no fountain review exists yet.

### Each model's pins

`--device` names the model whose pins every check in the session uses. Nothing of one model binds a session on the other: plans and journals record a kind naming the model, and a layout or plan of the other model is refused before anything is written.

| Pin | `plaf103` | `plwf106` |
| --- | --- | --- |
| Baseline | The research feeder's backup | Our fountain's backup, and its stock bank 0 and bank 1 by their own hashes |
| ROM report, for the writers and for `read small\|full\|status` | The feeder's stock console; a report written before the model was recorded counts as the feeder's | The fountain's stock console, named in the report |
| Updates | Bank 1 (`0x186000..0x2d6000`) or bank 0 | Bank 1 alone, `0x186000..0x271000`; without `--source`, the original bytes come from the backup |
| Give-way fallback | A frozen review (`--fallback`) | The backup's bank 0, byte for byte |
| Stock recovery window | `0x186000..0x2d6000` | `0x186000..0x271000` |
| Never written | The ranges in its [`layout.json`](../../src/firmware/devices/plaf103/layout.json) | The ranges in its [`layout.json`](../../src/firmware/devices/plwf106/layout.json), and bank 0 |

Every program frame is checked against the model's writable window and preserved ranges, on top of its bank's bounds, and a unit test holds the pins equal to each model's `layout.json`. The marker sequencing, preservation checks and stops are one code path for both models. The fountain's steps are on its [recovery page](../devices/plwf106/recovery.md).

## Output for scripts and agents

Progress goes to stderr; stdout carries exactly one JSON object: `status`, the output path, and the result or the error.

| Exit code | Meaning |
| --- | --- |
| 0 | Complete: inspected, prepared, verified, or a clean offline result |
| 1 | Stopped after the port opened; inspect the report and journal. Nothing was retried |
| 2 | Refused before any serial access: no port opened, and no output directory was created unless the error says otherwise |
| 3 | Incomplete: an `update --bank 0` wrote and verified bank 0, which has not recorded itself yet, so bank 1 refuses every update. The summary's `bank0.next` and stderr list the steps that finish it ([OTA design](../firmware/ota-design.md#bootstrap)) |

A command that opens the serial port first takes the [bench lease](../devices/plaf103/bench-server.md#bench-lease) under `--root`, or runs on its parent's when `wellfed-bench` started it. Another holder, or a lease a killed run left, is a refusal (exit 2) naming the holder. The offline commands take no lease.

Ctrl-C stops a session terminally and records the stop. It is checked before every transmission, so no frame follows the interrupt. A final report that cannot be saved after the port opened is a stop (exit 1, `report_error`), never a refusal.

## Common mistakes

- **Running from a worktree whose `private/` is a symlink.** The evidence paths don't resolve inside the checkout, so run the bench from a checkout whose `private/` is a real directory. The firmware build refuses there too, since the shared SDK tree records the main checkout's paths.

## Verification

- `cargo test -p wellfed-flash` runs every transaction against a modeled helper with a synthetic baseline, which only test builds accept: wire order checked against an independently restated policy, sector scope, lost ACKs, the interrupted mode, both banks, give-way with restore, stock recovery and refusals. A synthetic fountain runs an install, a give-way and a restore, and the fountain's refusals: bank 0, an oversize image, a preserved range, another backup and the feeder's layouts.
- `tests/emulated.rs`, in `make check-fixtures`, serves the pinned vendor helper, executed under Unicorn by `wellfed-emu`, on a Unix socket (`WELLFED_FLASH_EMULATOR`), and drives the built binary with the real baseline, frozen reviews and session reports:
  - update and give-way with restore: exactly the target is written, and the journal verifies;
  - a status read, and a refused serial-evidence continuation;
  - stock recovery: a full 8 MiB read, a reviewed plan and its execution, after which bank 1 is the baseline's and every other byte is unchanged;
  - a refused second planned erase inside the emulated part: the session stops (exit 1), resends nothing, and its journal never claims verification.
- With `WELLFED_FLASH_EMULATOR` set, reports record `"transport": "emulator"`, and a session continues only evidence from the same transport. Reports from serial sessions, the Python clients' included, count as serial.

Emulation runs the helper's command parser and packet checks as ARM code but models the ROM UART and flash calls, so it does not establish physical write acceptance, timing or power-cut behavior.

## Next steps

- [Development updates](uart-development.md): the update path in full.
- [Recovery](../devices/plaf103/recovery.md): the pinned recovery profile.
- [Optional fixtures](fixtures.md): the emulated tests' inputs.

## Evidence

- **Observed, October 22, 2026, research unit, relay bench driving power and the loader:** `read small` and `read status` matched a Python session byte for byte (data, bytes sent and bytes received). A no-op `update` erased nothing and verified the region. `give-way`, then `update --restore-marker`, booted bank 0 and then bank 1. `update --bank 0` installed a changed image: 140 sectors, verified. Until then the fixtures also required the Rust plan and wire to equal the Python updater's, and those passed.
- **Observed, October 23, 2026:** stock recovery ran on hardware in Rust. Bank 1 was restored to stock, and stock 1.1.15 booted. That session skipped the final full read at the owner's request; the next update inspection matched bank 1 against the frozen stock review instead.
- **Modeled, October 23, 2026:** the emulated stock recovery and fault cases passed on the research baseline. This is simulation only, not a hardware restore.
