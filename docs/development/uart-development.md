# Development updates over UART

This guide, for developers with a [bench](../devices/plaf103/bench.md) and a full backup, installs a reviewed build on a PLAF103 over UART, rewriting only the application sectors that changed. It holds only for the tested PLAF103 bank layout, works inside an already established, supervised helper session, and takes complete, frozen source and target images. The pinned stock-recovery writer keeps its original restrictions.

It is not over-the-air update, automatic rollback, a boot controller, or a way to resume a partial write. The factory guards still require the original research baseline, so another unit's backup is not automatically compatible.

## Freeze the images

`wellfed-build review` freezes either the exact stock OTA or a reviewed PLAF103 `firstboot` SDK build. Reviewing a build checks:

- the current source snapshot, SDK revision and overlays;
- the artifacts and archives, and both core packages;
- the linked mains and code, RAM parking and hooks, and stock pulls;
- the factory loader's own mapping and bank selection, executed under Unicorn (`wellfed-emu`) with the image alone in each bank.

A frozen review stays useful after later source changes; freezing a new application does not establish its behavior or physical acceptance. These commands are offline, open no serial port, and each writes to a new output directory.

1. Freeze the stock image:

   ```sh
   target/release/wellfed-build review stock --output private/verification/stock-review
   ```

2. Build and freeze the target:

   ```sh
   make -C src/firmware/vendor firstboot
   target/release/wellfed-build review build \
     --build private/firmware-build/plaf103/firstboot \
     --output private/verification/development-review
   ```

   You should see a review directory with `review.json` beside the frozen image.

3. List the update's arguments:

   ```sh
   cargo run --locked -p wellfed-flash -- update --device plaf103 --help
   ```

Keep a frozen source that matches the application actually installed. A build snapshot copied from the research archive can be stale against Wellfed's changed source and input hashes: rebuild and freeze a new target rather than editing manifests or substituting expected hashes.

## Run the update

The [Rust flash client](rust-flash.md) runs the update as `wellfed-flash update --device plaf103` in three modes, `inspect`, `prepare` and `execute`, each given explicit paths to:

- the baseline, and the frozen source and target directories;
- the port;
- the pinned ROM report and a fresh bounded-read report;
- new private output locations.

Reports and their companion serial and read files stay together.

1. Run `wellfed-flash update --device plaf103 inspect` with those inputs. It reads the device and writes nothing. You should see an inspection report; the current application must exactly match the frozen source, and the factory range the pinned baseline.
2. Run `wellfed-flash update --device plaf103 prepare` on that inspection. It opens no port, binds the complete inspected source region, the report and the tool hashes into a plan, and prints the plan's SHA-256.
3. Check the plan before you write: the bank, the source and target reviews, and the sectors it will touch. Confirm the unit has a full backup.

   > [!WARNING]
   > The next step erases and programs flash. A wrong plan, a wrong unit or a stop at the wrong moment can leave the bank unbootable.

4. Run `wellfed-flash update --device plaf103 execute` with exactly that plan and its SHA-256.

   You should see the final verification pass and exit 0. Exit 1 means it stopped after the port opened: read the report and journal, and see [finishing an interrupted update](#finish-an-interrupted-update).

## What the updater writes

**Bank 1** is the default. Writable bank 1 starts at `0x186000` and cannot extend beyond `0x2d6000`.

Each acquisition reads the union of the old and new application extents, plus the 24 KiB factory range and the other bank's 4 KiB first sector, whose hash the plan pins, so it must stay byte-identical across every acquisition. For bank 1, bank 0 must be either the pinned baseline (an invalidated marker) or a valid image. A valid bank 0 is safer: the factory loader prefers bank 1 and falls back to bank 0 ([bank policy](../firmware/bank-policy.md)), so an interrupted bank-1 update boots bank 0 rather than waiting forever.

**`--bank 0`** updates bank 0 (`0x6000`–`0x156000`, the same reviewed footprint) under its own transaction kind. Bank 1 must be valid throughout, so the unit always has a bootable bank while bank 0's marker is withheld. Bank 0 may omit `--source` if it still holds exactly the pinned baseline's bytes over the target's extent, with an invalid marker; the target overwrites it, marker last.

In both modes the updater itself bounds program packets to the selected bank, and the factory range `0x0`–`0x6000` is never a write target.

### The write sequence

For a changed image, execution:

1. verifies the preimage application region and guards;
2. stages the marker sector first, with eight erased bytes;
3. stages only the other changed sectors, with status checks, durable intent and exact sector readback;
4. verifies the whole region;
5. commits the marker with one 1 KiB no-erase program;
6. verifies again.

A shorter target keeps and verifies the old tail, a larger one explicitly includes the additional sectors, and a no-op plan performs one acquisition and no writes.

Readiness synchronization follows the durable erase-intent fsync, immediately before transmission, so a journal stall cannot cross the helper's idle-ready interval.

The helper sends a NAK every 50 ms while it waits for a command, and a NAK still on the wire when a command reaches it is read as the reply, which stops the session. So a command must leave just after a NAK arrives. A development update and a read drop the NAKs already buffered (any other byte stops the session) and go on the next NAK: about 25 ms a command where three NAKs took 100 to 150 ms, and never on NAKs queued during a host pause, which is what stopped earlier installs on a loader `0x15`. The stock, recovery and give-way writers keep the three-NAK wait every write was first proven with.

Missing ACKs, protocol errors or drift stop the session terminally, with no automatic resume, replay, fallback write or boot command.

### What it doesn't reread

Settings, audio and log areas outside the declared application footprint are not reread during each update. Their preservation relies on the bounded mutation sequence, narrower evidence than a full-flash preservation audit, which layout changes, recovery investigations and unexplained drift need.

## Give way to bank 0

`wellfed-flash give-way --device plaf103` makes the loader fall back to bank 0, and runs only when bank 0 holds exactly a frozen, reviewed image, read in full.

> [!WARNING]
> Give-way programs flash. Check that bank 0 holds the reviewed image you expect before you execute.

It clears bank 1's first marker byte with one 1 KiB program frame that rewrites the other 1023 bytes unchanged, since NOR programming only clears bits. Stock's own activation does much the same over the marker's whole first word (physical observation, October 29, 2026).

To bring bank 1 back, run a development update with `--restore-marker`. It accepts only the source with its first marker byte cleared, rewrites that one sector with the marker withheld, then commits the marker last.

## Finish an interrupted update

`--interrupted` finishes a stopped update from a fresh inspection, never from the stopped journal. The flag is bound into the inspection, plan and execution, so a normal inspection cannot prepare an interrupted plan, nor the reverse. It accepts only these states:

- The marker sector is staged, and every other sector is exactly its source, its staged target, or erased.
- One sector may instead hold the staged target's first one to three 1024-byte program frames followed by erased flash, the only state a program stopped after its erase leaves. That sector is erased and rewritten.
- A stop between the marker sector's erase and its first program frame: the marker sector wholly erased (every byte `0xFF`), and every other sector in the extent byte-identical to the source review. The plan then stages the marker sector first and commits the marker last, as a fresh update does.

Anything else is refused: a partial erase, a second partial program, stray bytes or foreign frames in the marker sector, or any other content. The other bank's pinned first sector is checked as always, and the update then stages the remaining sectors and commits the marker with a normal update's checks.

## Not yet established

- Returning to stock through this updater is untested. Don't infer it from emulator success or from the full-image stock restoration.
- Power-cut and failed-boot acceptance are open.
- Neither a roadmap status nor a copied historical plan authorizes a new physical operation.

## Next steps

- [Rust flash client](rust-flash.md): exit codes, the bench lease and the other commands.
- [UART and loader](../devices/plaf103/uart.md) and [recovery limits](../devices/plaf103/recovery.md).
- [Startup diagnostics](../firmware/startup-diagnostics.md) and [optional fixtures](fixtures.md).

## Evidence

- **Observed:** changed-build updates and their following boots on the research unit. One small-image iteration took about **2m22s** for inspection plus execution, excluding build, manual setup and normal boot; timing depends on the image changes and hardware, and a small source edit can move many linked sectors.
- **Observed:** the conservative full-image stock restoration and a following stock 1.1.15 boot were completed separately, not through this updater.
